use crate::{Error, ErrorKind, PathSegment, Value, ValueRef, MAX_DEPTH};

pub fn encode(value: &Value) -> Result<Vec<u8>, Error> {
    let mut encoder = Encoder {
        output: Vec::new(),
        path: Vec::new(),
    };
    encoder.value(value)?;
    Ok(encoder.output)
}

/// Encodes a borrowed canonical value.
///
/// The returned buffer is not zeroized automatically. Callers encoding
/// sensitive fields should immediately move it into zeroizing storage.
pub fn encode_ref(value: &ValueRef<'_>) -> Result<Vec<u8>, Error> {
    let mut encoder = Encoder {
        output: Vec::new(),
        path: Vec::new(),
    };
    encoder.value_ref(value)?;
    Ok(encoder.output)
}

struct Encoder {
    output: Vec<u8>,
    path: Vec<PathSegment>,
}

impl Encoder {
    fn error(&self, kind: ErrorKind) -> Error {
        Error::new(kind, self.output.len(), &self.path)
    }

    fn value(&mut self, value: &Value) -> Result<(), Error> {
        if self.path.len() > MAX_DEPTH {
            return Err(self.error(ErrorKind::DepthLimit));
        }
        match value {
            Value::Null => self.output.push(0xc0),
            Value::Bool(false) => self.output.push(0xc2),
            Value::Bool(true) => self.output.push(0xc3),
            Value::Unsigned(value) => self.unsigned(*value),
            Value::Negative(value) => self.negative(*value)?,
            Value::Binary(bytes) => self.binary(bytes)?,
            Value::Text(bytes) => self.text(bytes)?,
            Value::Array(values) => self.array(values)?,
            Value::Variant(value) => self.variant(value.as_ref())?,
        }
        Ok(())
    }

    fn value_ref(&mut self, value: &ValueRef<'_>) -> Result<(), Error> {
        if self.path.len() > MAX_DEPTH {
            return Err(self.error(ErrorKind::DepthLimit));
        }
        match value {
            ValueRef::Value(value) => self.value(value)?,
            ValueRef::Null => self.output.push(0xc0),
            ValueRef::Bool(false) => self.output.push(0xc2),
            ValueRef::Bool(true) => self.output.push(0xc3),
            ValueRef::Unsigned(value) => self.unsigned(*value),
            ValueRef::Negative(value) => self.negative(*value)?,
            ValueRef::Binary(bytes) => self.binary(bytes)?,
            ValueRef::Text(bytes) => self.text(bytes)?,
            ValueRef::Array(values) => self.array_ref(values)?,
            ValueRef::Variant(value) => self.variant_ref(value.as_ref())?,
        }
        Ok(())
    }

    fn unsigned(&mut self, value: u64) {
        match value {
            0..=0x7f => self.output.push(value as u8),
            0x80..=0xff => self.output.extend([0xcc, value as u8]),
            0x100..=0xffff => {
                self.output.push(0xcd);
                self.output.extend((value as u16).to_be_bytes());
            }
            0x1_0000..=0xffff_ffff => {
                self.output.push(0xce);
                self.output.extend((value as u32).to_be_bytes());
            }
            _ => {
                self.output.push(0xcf);
                self.output.extend(value.to_be_bytes());
            }
        }
    }

    fn negative(&mut self, value: i64) -> Result<(), Error> {
        if value >= 0 {
            return Err(self.error(ErrorKind::NonNegativeSignedInteger));
        }
        match value {
            -32..=-1 => self.output.push(value as i8 as u8),
            -128..=-33 => self.output.extend([0xd0, value as i8 as u8]),
            -32_768..=-129 => {
                self.output.push(0xd1);
                self.output.extend((value as i16).to_be_bytes());
            }
            -2_147_483_648..=-32_769 => {
                self.output.push(0xd2);
                self.output.extend((value as i32).to_be_bytes());
            }
            _ => {
                self.output.push(0xd3);
                self.output.extend(value.to_be_bytes());
            }
        }
        Ok(())
    }

    fn binary(&mut self, bytes: &[u8]) -> Result<(), Error> {
        write_len_header(&mut self.output, bytes.len(), 0xc4, 0xc5, 0xc6, 0, 0)?;
        self.output.extend(bytes);
        Ok(())
    }

    fn text(&mut self, bytes: &[u8]) -> Result<(), Error> {
        match bytes.len() {
            length @ 0..=31 => self.output.push(0xa0 | length as u8),
            length @ 32..=255 => self.output.extend([0xd9, length as u8]),
            length @ 256..=65_535 => {
                self.output.push(0xda);
                self.output.extend((length as u16).to_be_bytes());
            }
            length => {
                let length =
                    u32::try_from(length).map_err(|_| self.error(ErrorKind::LengthOverflow))?;
                self.output.push(0xdb);
                self.output.extend(length.to_be_bytes());
            }
        }
        self.output.extend(bytes);
        Ok(())
    }

    fn array(&mut self, values: &[Value]) -> Result<(), Error> {
        self.array_header(values.len())?;
        for (index, value) in values.iter().enumerate() {
            self.path.push(PathSegment::Index(index));
            let result = self.value(value);
            self.path.pop();
            result?;
        }
        Ok(())
    }

    fn array_ref(&mut self, values: &[ValueRef<'_>]) -> Result<(), Error> {
        self.array_header(values.len())?;
        for (index, value) in values.iter().enumerate() {
            self.path.push(PathSegment::Index(index));
            let result = self.value_ref(value);
            self.path.pop();
            result?;
        }
        Ok(())
    }

    fn array_header(&mut self, length: usize) -> Result<(), Error> {
        match length {
            0 => return Err(self.error(ErrorKind::EmptyArray)),
            length @ 1..=15 => self.output.push(0x90 | length as u8),
            length @ 16..=65_535 => {
                self.output.push(0xdc);
                self.output.extend((length as u16).to_be_bytes());
            }
            length => {
                let length =
                    u32::try_from(length).map_err(|_| self.error(ErrorKind::LengthOverflow))?;
                self.output.push(0xdd);
                self.output.extend(length.to_be_bytes());
            }
        }
        Ok(())
    }

    fn variant(&mut self, value: Option<&(Vec<u8>, Box<Value>)>) -> Result<(), Error> {
        let Some((tag, value)) = value else {
            self.output.push(0x80);
            return Ok(());
        };
        if tag.len() > 31 {
            return Err(self.error(ErrorKind::InvalidVariantTag));
        }
        self.output.extend([0x81, 0xa0 | tag.len() as u8]);
        self.output.extend(tag);
        self.path.push(PathSegment::Variant(tag.clone()));
        let result = self.value(value);
        self.path.pop();
        result
    }

    fn variant_ref(&mut self, value: Option<&(&[u8], Box<ValueRef<'_>>)>) -> Result<(), Error> {
        let Some((tag, value)) = value else {
            self.output.push(0x80);
            return Ok(());
        };
        if tag.len() > 31 {
            return Err(self.error(ErrorKind::InvalidVariantTag));
        }
        self.output.extend([0x81, 0xa0 | tag.len() as u8]);
        self.output.extend(*tag);
        self.path.push(PathSegment::Variant(tag.to_vec()));
        let result = self.value_ref(value);
        self.path.pop();
        result
    }
}

fn write_len_header(
    output: &mut Vec<u8>,
    length: usize,
    marker8: u8,
    marker16: u8,
    marker32: u8,
    min8: usize,
    min16: usize,
) -> Result<(), Error> {
    match length {
        length if length >= min8 && length <= 255 => output.extend([marker8, length as u8]),
        length if length >= min16 && length <= 65_535 => {
            output.push(marker16);
            output.extend((length as u16).to_be_bytes());
        }
        length => {
            let length = u32::try_from(length)
                .map_err(|_| Error::new(ErrorKind::LengthOverflow, output.len(), &[]))?;
            output.push(marker32);
            output.extend(length.to_be_bytes());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integer_boundaries_use_the_shortest_marker() {
        let cases = [
            (Value::Unsigned(0), vec![0x00]),
            (Value::Unsigned(127), vec![0x7f]),
            (Value::Unsigned(128), vec![0xcc, 0x80]),
            (Value::Unsigned(255), vec![0xcc, 0xff]),
            (Value::Unsigned(256), vec![0xcd, 0x01, 0x00]),
            (Value::Unsigned(65_535), vec![0xcd, 0xff, 0xff]),
            (Value::Unsigned(65_536), vec![0xce, 0x00, 0x01, 0x00, 0x00]),
            (Value::Negative(-1), vec![0xff]),
            (Value::Negative(-32), vec![0xe0]),
            (Value::Negative(-33), vec![0xd0, 0xdf]),
            (Value::Negative(-128), vec![0xd0, 0x80]),
            (Value::Negative(-129), vec![0xd1, 0xff, 0x7f]),
        ];
        for (value, expected) in cases {
            assert_eq!(encode(&value).unwrap(), expected);
        }
    }

    #[test]
    fn nonnegative_negative_variant_is_rejected() {
        assert!(matches!(
            encode(&Value::Negative(0)).unwrap_err().kind,
            ErrorKind::NonNegativeSignedInteger
        ));
    }

    #[test]
    fn empty_arrays_are_rejected_and_array16_is_minimal() {
        assert!(matches!(
            encode(&Value::Array(vec![])).unwrap_err().kind,
            ErrorKind::EmptyArray
        ));
        for length in 16..=31 {
            let values = vec![Value::Null; length];
            let encoded = encode(&Value::Array(values)).unwrap();
            assert_eq!(&encoded[..3], &[0xdc, 0, length as u8]);
        }
    }

    #[test]
    fn borrowed_values_match_owned_encoding() {
        let owned = Value::Array(vec![
            Value::Unsigned(7),
            Value::Binary(vec![1, 2, 3]),
            Value::Variant(Some((
                b"x".to_vec(),
                Box::new(Value::Text(b"sensitive".to_vec())),
            ))),
        ]);
        let borrowed = ValueRef::from(&owned);
        assert_eq!(encode_ref(&borrowed).unwrap(), encode(&owned).unwrap());

        let secret = [9_u8; 32];
        assert_eq!(
            encode_ref(&ValueRef::Binary(&secret)).unwrap(),
            encode(&Value::Binary(secret.to_vec())).unwrap()
        );
    }
}
