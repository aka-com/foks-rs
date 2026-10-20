//! Strict scalar host data; no untrusted error formatting or owned map keys.
use super::MAX_BYTES;
use crate::scalar::Decimal;
use crate::{
    catalog::{Kind, Spec},
    Error, Result as CoreResult, Value, MAX_BLOB_BYTES,
};
use serde::de::{self, DeserializeSeed, Deserializer, MapAccess, SeqAccess, Visitor};
use std::{cell::Cell, fmt, mem::size_of};
const MESSAGE: &str = "invalid scalar response";
fn invalid<E: de::Error>() -> E {
    E::custom(MESSAGE)
}
trait Shape<'de>: Sized {
    type Value;
    fn unsigned<E: de::Error>(self, _: u64) -> Result<Self::Value, E> {
        Err(invalid())
    }
    fn text<E: de::Error>(self, _: &str) -> Result<Self::Value, E> {
        Err(invalid())
    }
    fn unit<E: de::Error>(self) -> Result<Self::Value, E> {
        Err(invalid())
    }
    fn seq<A: SeqAccess<'de>>(self, _: A) -> Result<Self::Value, A::Error> {
        Err(invalid())
    }
    fn map<A: MapAccess<'de>>(self, _: A) -> Result<Self::Value, A::Error> {
        Err(invalid())
    }
}
struct Strict<S>(S);
impl<'de, S: Shape<'de>> Visitor<'de> for Strict<S> {
    type Value = S::Value;
    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("bounded host context")
    }
    fn visit_u64<E: de::Error>(self, value: u64) -> Result<Self::Value, E> {
        self.0.unsigned(value)
    }
    fn visit_u8<E: de::Error>(self, value: u8) -> Result<Self::Value, E> {
        self.0.unsigned(u64::from(value))
    }
    fn visit_u16<E: de::Error>(self, value: u16) -> Result<Self::Value, E> {
        self.0.unsigned(u64::from(value))
    }
    fn visit_u32<E: de::Error>(self, value: u32) -> Result<Self::Value, E> {
        self.0.unsigned(u64::from(value))
    }
    fn visit_u128<E: de::Error>(self, value: u128) -> Result<Self::Value, E> {
        self.0
            .unsigned(value.try_into().map_err(|_| invalid::<E>())?)
    }
    fn visit_borrowed_str<E: de::Error>(self, value: &'de str) -> Result<Self::Value, E> {
        self.0.text(value)
    }
    fn visit_seq<A: SeqAccess<'de>>(self, value: A) -> Result<Self::Value, A::Error> {
        self.0.seq(value)
    }
    fn visit_map<A: MapAccess<'de>>(self, value: A) -> Result<Self::Value, A::Error> {
        self.0.map(value)
    }
    fn visit_bool<E: de::Error>(self, _: bool) -> Result<Self::Value, E> {
        Err(invalid())
    }
    fn visit_i8<E: de::Error>(self, _: i8) -> Result<Self::Value, E> {
        Err(invalid())
    }
    fn visit_i16<E: de::Error>(self, _: i16) -> Result<Self::Value, E> {
        Err(invalid())
    }
    fn visit_i32<E: de::Error>(self, _: i32) -> Result<Self::Value, E> {
        Err(invalid())
    }
    fn visit_i64<E: de::Error>(self, _: i64) -> Result<Self::Value, E> {
        Err(invalid())
    }
    fn visit_i128<E: de::Error>(self, _: i128) -> Result<Self::Value, E> {
        Err(invalid())
    }
    fn visit_f32<E: de::Error>(self, _: f32) -> Result<Self::Value, E> {
        Err(invalid())
    }
    fn visit_f64<E: de::Error>(self, _: f64) -> Result<Self::Value, E> {
        Err(invalid())
    }
    fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
        self.0.text(value)
    }
    fn visit_string<E: de::Error>(self, _: String) -> Result<Self::Value, E> {
        Err(invalid())
    }
    fn visit_bytes<E: de::Error>(self, _: &[u8]) -> Result<Self::Value, E> {
        Err(invalid())
    }
    fn visit_borrowed_bytes<E: de::Error>(self, _: &'de [u8]) -> Result<Self::Value, E> {
        Err(invalid())
    }
    fn visit_byte_buf<E: de::Error>(self, _: Vec<u8>) -> Result<Self::Value, E> {
        Err(invalid())
    }
    fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
        Err(invalid())
    }
    fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
        self.0.unit()
    }
    fn visit_some<D: Deserializer<'de>>(self, _: D) -> Result<Self::Value, D::Error> {
        Err(invalid())
    }
    fn visit_newtype_struct<D: Deserializer<'de>>(self, _: D) -> Result<Self::Value, D::Error> {
        Err(invalid())
    }
}

#[derive(Default)]
struct Admission {
    refused: Cell<bool>,
}
impl Admission {
    fn fail<E: de::Error>(&self) -> E {
        self.refused.set(true);
        invalid()
    }
    fn vector<T, E: de::Error>(&self, count: usize) -> std::result::Result<Vec<T>, E> {
        let bytes = count
            .checked_mul(size_of::<T>())
            .ok_or_else(|| self.fail::<E>())?;
        if bytes != 0 {
            crate::page::charge(bytes).map_err(|_| self.fail::<E>())?;
        }
        let mut values = Vec::new();
        values
            .try_reserve_exact(count)
            .map_err(|_| self.fail::<E>())?;
        Ok(values)
    }
    fn copy<E: de::Error>(&self, text: &str) -> std::result::Result<String, E> {
        let mut bytes = self.vector::<u8, E>(text.len())?;
        bytes.extend_from_slice(text.as_bytes());
        String::from_utf8(bytes).map_err(|_| invalid())
    }
}

struct Reject;
impl<'de> Shape<'de> for Reject {
    type Value = ();
}
impl<'de> DeserializeSeed<'de> for Reject {
    type Value = ();
    fn deserialize<D: Deserializer<'de>>(self, d: D) -> std::result::Result<(), D::Error> {
        d.deserialize_any(Strict(self))
    }
}
struct Key(&'static str);
impl<'de> Shape<'de> for Key {
    type Value = ();
    fn text<E: de::Error>(self, text: &str) -> std::result::Result<(), E> {
        if text == self.0 {
            Ok(())
        } else {
            Err(invalid())
        }
    }
}
impl<'de> DeserializeSeed<'de> for Key {
    type Value = ();
    fn deserialize<D: Deserializer<'de>>(self, d: D) -> std::result::Result<(), D::Error> {
        d.deserialize_any(Strict(self))
    }
}
struct Byte;
impl<'de> Shape<'de> for Byte {
    type Value = u8;
    fn unsigned<E: de::Error>(self, n: u64) -> std::result::Result<u8, E> {
        n.try_into().map_err(|_| invalid())
    }
}
impl<'de> DeserializeSeed<'de> for Byte {
    type Value = u8;
    fn deserialize<D: Deserializer<'de>>(self, d: D) -> std::result::Result<u8, D::Error> {
        d.deserialize_any(Strict(self))
    }
}
struct Body<'a> {
    kind: Kind,
    admission: &'a Admission,
}
impl<'de> Shape<'de> for Body<'_> {
    type Value = Value;
    fn text<E: de::Error>(self, text: &str) -> std::result::Result<Value, E> {
        match self.kind {
            Kind::Integer => {
                if text.len() > 20 {
                    return Err(invalid());
                }
                let n: i64 = text.parse().map_err(|_| invalid::<E>())?;
                if Decimal::new(n).bytes() != text.as_bytes() {
                    return Err(invalid());
                }
                Ok(Value::Integer(n))
            }
            Kind::Text if text.len() <= MAX_BLOB_BYTES => {
                Ok(Value::Text(self.admission.copy(text)?))
            }
            _ => Err(invalid()),
        }
    }
    fn seq<A: SeqAccess<'de>>(self, mut seq: A) -> std::result::Result<Value, A::Error> {
        if !matches!(self.kind, Kind::Blob | Kind::NullableBlob) {
            return Err(invalid());
        }
        let mut bytes = self.admission.vector::<u8, A::Error>(MAX_BLOB_BYTES)?;
        while let Some(byte) = seq.next_element_seed(Byte)? {
            if bytes.len() == MAX_BLOB_BYTES {
                return Err(invalid());
            }
            bytes.push(byte);
        }
        Ok(Value::Blob(bytes))
    }
}
impl<'de> DeserializeSeed<'de> for Body<'_> {
    type Value = Value;
    fn deserialize<D: Deserializer<'de>>(self, d: D) -> std::result::Result<Value, D::Error> {
        d.deserialize_any(Strict(self))
    }
}
struct Column<'a> {
    kind: Kind,
    admission: &'a Admission,
}
impl<'de> Shape<'de> for Column<'_> {
    type Value = Value;
    fn text<E: de::Error>(self, text: &str) -> std::result::Result<Value, E> {
        if matches!(self.kind, Kind::NullableBlob) && text == "null" {
            Ok(Value::Null)
        } else {
            Err(invalid())
        }
    }
    fn map<A: MapAccess<'de>>(self, mut map: A) -> std::result::Result<Value, A::Error> {
        let name = match self.kind {
            Kind::Integer => "integer",
            Kind::Text => "text",
            Kind::Blob | Kind::NullableBlob => "blob",
        };
        map.next_key_seed(Key(name))?
            .ok_or_else(invalid::<A::Error>)?;
        let value = map.next_value_seed(Body {
            kind: self.kind,
            admission: self.admission,
        })?;
        if map.next_key_seed(Reject)?.is_some() {
            return Err(invalid());
        }
        Ok(value)
    }
}
impl<'de> DeserializeSeed<'de> for Column<'_> {
    type Value = Value;
    fn deserialize<D: Deserializer<'de>>(self, d: D) -> std::result::Result<Value, D::Error> {
        d.deserialize_any(Strict(self))
    }
}
struct Query<'a> {
    columns: &'a [Kind],
    admission: &'a Admission,
}
impl<'de> Shape<'de> for Query<'_> {
    type Value = Option<Vec<Value>>;
    fn unit<E: de::Error>(self) -> std::result::Result<Self::Value, E> {
        Ok(None)
    }
    fn seq<A: SeqAccess<'de>>(self, mut seq: A) -> std::result::Result<Self::Value, A::Error> {
        let mut values = self
            .admission
            .vector::<Value, A::Error>(self.columns.len())?;
        for kind in self.columns {
            values.push(
                seq.next_element_seed(Column {
                    kind: *kind,
                    admission: self.admission,
                })?
                .ok_or_else(invalid::<A::Error>)?,
            );
        }
        if seq.next_element_seed(Reject)?.is_some() {
            return Err(invalid());
        }
        Ok(Some(values))
    }
}
impl<'de> DeserializeSeed<'de> for Query<'_> {
    type Value = Option<Vec<Value>>;
    fn deserialize<D: Deserializer<'de>>(self, d: D) -> std::result::Result<Self::Value, D::Error> {
        d.deserialize_any(Strict(self))
    }
}

fn preflight(bytes: &[u8]) -> CoreResult<()> {
    let mut depth = 0usize;
    let mut stack = [0u8; 3];
    let mut quoted = false;
    let mut at = 0;
    while at < bytes.len() {
        let byte = bytes[at];
        if quoted {
            match byte {
                b'"' => quoted = false,
                b'\\' => {
                    // Skip the escaped byte only. Serde validates the full escape,
                    // UTF-8 and surrogate pair without changing lexical nesting.
                    at += 1;
                    if at == bytes.len() {
                        return Err(Error::Storage);
                    }
                }
                0..=31 => return Err(Error::Storage),
                _ => (),
            }
        } else {
            match byte {
                b'"' => quoted = true,
                b'[' | b'{' => {
                    if depth == stack.len() {
                        return Err(Error::Storage);
                    }
                    stack[depth] = byte;
                    depth += 1;
                }
                b']' | b'}' => {
                    if depth == 0 || stack[depth - 1] != if byte == b']' { b'[' } else { b'{' } {
                        return Err(Error::Storage);
                    }
                    depth -= 1;
                }
                b'0'..=b'9' => {
                    let start = at;
                    let mut n = 0u16;
                    while at < bytes.len() && bytes[at].is_ascii_digit() {
                        if at - start == 3 {
                            return Err(Error::Storage);
                        }
                        n = n * 10 + u16::from(bytes[at] - b'0');
                        at += 1;
                    }
                    if n > 255 || (at - start > 1 && byte == b'0') {
                        return Err(Error::Storage);
                    }
                    continue;
                }
                b'n' if bytes.get(at..at + 4) == Some(b"null") => {
                    at += 4;
                    continue;
                }
                b' ' | b'\n' | b'\r' | b'\t' | b',' | b':' => (),
                _ => return Err(Error::Storage),
            }
        }
        at += 1;
    }
    if quoted || depth != 0 {
        return Err(Error::Storage);
    }
    Ok(())
}
fn prepare(bytes: &[u8]) -> CoreResult<()> {
    if bytes.len() > MAX_BYTES {
        return Err(Error::Bounds);
    }
    preflight(bytes)?;
    // Held live scratch/error allowance, not cumulative parser allocator traffic.
    let bytes = bytes
        .len()
        .checked_mul(3)
        .and_then(|n| n.checked_add(2344))
        .ok_or(Error::Bounds)?;
    crate::page::charge(bytes)
}
fn parse<'de, S: DeserializeSeed<'de>>(
    bytes: &'de [u8],
    seed: S,
    admission: &Admission,
) -> CoreResult<S::Value> {
    let mut decoder = serde_json::Deserializer::from_slice(bytes);
    let result = seed.deserialize(&mut decoder).and_then(|value| {
        decoder.end()?;
        Ok(value)
    });
    if admission.refused.get() {
        return Err(Error::Bounds);
    }
    result.map_err(|_| Error::Storage)
}
pub(super) fn query(spec: &Spec, bytes: &[u8]) -> CoreResult<Option<Vec<Value>>> {
    prepare(bytes)?;
    let admission = Admission::default();
    parse(
        bytes,
        Query {
            columns: spec.columns,
            admission: &admission,
        },
        &admission,
    )
}
pub(super) fn changes(bytes: &[u8]) -> CoreResult<u64> {
    prepare(bytes)?;
    let admission = Admission::default();
    let value = parse(
        bytes,
        Column {
            kind: Kind::Integer,
            admission: &admission,
        },
        &admission,
    )?;
    crate::unsigned(&value)
}

const _: () = assert!(
    size_of::<Query<'_>>()
        + size_of::<Column<'_>>()
        + size_of::<Body<'_>>()
        + size_of::<Admission>()
        + size_of::<Strict<Query<'_>>>()
        <= 2048
);
const _: () = assert!(MESSAGE.len() <= 64);

#[cfg(test)]
pub(super) fn diagnostic(bytes: &[u8]) -> String {
    let admission = Admission::default();
    let mut decoder = serde_json::Deserializer::from_slice(bytes);
    Query {
        columns: &[Kind::Text],
        admission: &admission,
    }
    .deserialize(&mut decoder)
    .unwrap_err()
    .to_string()
}
