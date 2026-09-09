use crate::{Error, ErrorKind, PathSegment, Value, MAX_DEPTH};

// Snowpack frames are network-facing. Cap the total decoded AST, rather than
// only each array, because small encoded scalars expand substantially as
// `Value` nodes and nested arrays can otherwise multiply that amplification.
const MAXIMUM_DECODED_VALUES: usize = 1_000_000;

pub fn decode(input: &[u8]) -> Result<Value, Error> {
    let (value, consumed) = decode_prefix(input)?;
    if consumed != input.len() {
        return Err(Error::new(ErrorKind::TrailingBytes, consumed, &[]));
    }
    Ok(value)
}

/// Decodes one canonical value at the start of `input` and returns the number
/// of bytes consumed. FOKS uses this for authenticated plaintexts whose
/// canonical Snowpack value is followed by zero padding.
pub fn decode_prefix(input: &[u8]) -> Result<(Value, usize), Error> {
    let mut decoder = Decoder::new(input, false);
    let value = decoder.value()?;
    Ok((value, decoder.offset))
}

pub fn validate(input: &[u8]) -> Result<(), Error> {
    decode(input).map(|_| ())
}

/// Validates that an encoding satisfies strict canonical rules for signed,
/// verified, and hashed objects. Rejects `array16` encodings with 16 to 31
/// elements, which are reserved for fixarray representation in signable contexts.
pub fn validate_signable(input: &[u8]) -> Result<(), Error> {
    let mut decoder = Decoder::new(input, true);
    decoder.value()?;
    if decoder.offset != input.len() {
        return Err(Error::new(ErrorKind::TrailingBytes, decoder.offset, &[]));
    }
    Ok(())
}

struct Decoder<'a> {
    input: &'a [u8],
    offset: usize,
    path: Vec<PathSegment>,
    values: usize,
    /// Enforce go-foks's stricter canonical rules for signable encodings.
    signable: bool,
}

impl<'a> Decoder<'a> {
    fn new(input: &'a [u8], signable: bool) -> Self {
        Self {
            input,
            offset: 0,
            path: Vec::new(),
            values: 0,
            signable,
        }
    }
}

impl Decoder<'_> {
    fn error(&self, kind: ErrorKind) -> Error {
        Error::new(kind, self.offset, &self.path)
    }

    fn byte(&mut self) -> Result<u8, Error> {
        let byte = self
            .input
            .get(self.offset)
            .copied()
            .ok_or_else(|| self.error(ErrorKind::UnexpectedEof))?;
        self.offset += 1;
        Ok(byte)
    }

    fn bytes(&mut self, length: usize) -> Result<Vec<u8>, Error> {
        let end = self
            .offset
            .checked_add(length)
            .ok_or_else(|| self.error(ErrorKind::LengthOverflow))?;
        let bytes = self
            .input
            .get(self.offset..end)
            .ok_or_else(|| self.error(ErrorKind::UnexpectedEof))?
            .to_vec();
        self.offset = end;
        Ok(bytes)
    }

    fn u16(&mut self) -> Result<u16, Error> {
        let bytes: [u8; 2] = self.bytes(2)?.try_into().expect("length checked by bytes");
        Ok(u16::from_be_bytes(bytes))
    }

    fn u32(&mut self) -> Result<u32, Error> {
        let bytes: [u8; 4] = self.bytes(4)?.try_into().expect("length checked by bytes");
        Ok(u32::from_be_bytes(bytes))
    }

    fn u64(&mut self) -> Result<u64, Error> {
        let bytes: [u8; 8] = self.bytes(8)?.try_into().expect("length checked by bytes");
        Ok(u64::from_be_bytes(bytes))
    }

    fn value(&mut self) -> Result<Value, Error> {
        self.values = self
            .values
            .checked_add(1)
            .ok_or_else(|| self.error(ErrorKind::ValueLimit))?;
        if self.values > MAXIMUM_DECODED_VALUES {
            return Err(self.error(ErrorKind::ValueLimit));
        }
        if self.path.len() > MAX_DEPTH {
            return Err(self.error(ErrorKind::DepthLimit));
        }
        let marker = self.byte()?;
        match marker {
            0x00..=0x7f => Ok(Value::Unsigned(u64::from(marker))),
            0x80 => Ok(Value::Variant(None)),
            0x81 => self.variant(),
            0x82..=0x8f => Err(self.error(ErrorKind::InvalidVariantSize((marker & 0x0f) as usize))),
            0x90 => Err(self.error(ErrorKind::EmptyArray)),
            0x91..=0x9f => self.array((marker & 0x0f) as usize),
            0xa0..=0xbf => self.text((marker & 0x1f) as usize),
            0xc0 => Ok(Value::Null),
            0xc1 => Err(self.error(ErrorKind::ReservedMarker(marker))),
            0xc2 => Ok(Value::Bool(false)),
            0xc3 => Ok(Value::Bool(true)),
            0xc4 => {
                let length = usize::from(self.byte()?);
                self.binary(length)
            }
            0xc5 => {
                let length = usize::from(self.u16()?);
                if length <= 255 {
                    return Err(self.error(ErrorKind::NonMinimal("binary")));
                }
                self.binary(length)
            }
            0xc6 => {
                let length = usize::try_from(self.u32()?)
                    .map_err(|_| self.error(ErrorKind::LengthOverflow))?;
                if length <= 65_535 {
                    return Err(self.error(ErrorKind::NonMinimal("binary")));
                }
                self.binary(length)
            }
            0xc7..=0xc9 | 0xd4..=0xd8 => Err(self.error(ErrorKind::UnsupportedExtension)),
            0xca..=0xcb => Err(self.error(ErrorKind::UnsupportedFloat)),
            0xcc => {
                let value = self.byte()?;
                if value <= 0x7f {
                    return Err(self.error(ErrorKind::NonMinimal("unsigned integer")));
                }
                Ok(Value::Unsigned(u64::from(value)))
            }
            0xcd => {
                let value = self.u16()?;
                if value <= u16::from(u8::MAX) {
                    return Err(self.error(ErrorKind::NonMinimal("unsigned integer")));
                }
                Ok(Value::Unsigned(u64::from(value)))
            }
            0xce => {
                let value = self.u32()?;
                if value <= u32::from(u16::MAX) {
                    return Err(self.error(ErrorKind::NonMinimal("unsigned integer")));
                }
                Ok(Value::Unsigned(u64::from(value)))
            }
            0xcf => {
                let value = self.u64()?;
                if value <= u64::from(u32::MAX) {
                    return Err(self.error(ErrorKind::NonMinimal("unsigned integer")));
                }
                Ok(Value::Unsigned(value))
            }
            0xd0 => {
                let value = i64::from(self.byte()? as i8);
                self.signed(value, i64::from(i8::MIN))
            }
            0xd1 => {
                let value = i64::from(self.u16()? as i16);
                self.signed(value, i64::from(i16::MIN))
            }
            0xd2 => {
                let value = i64::from(self.u32()? as i32);
                self.signed(value, i64::from(i32::MIN))
            }
            0xd3 => {
                let value = self.u64()? as i64;
                if value >= i64::from(i32::MIN) {
                    return Err(self.error(if value >= 0 {
                        ErrorKind::NonNegativeSignedInteger
                    } else {
                        ErrorKind::NonMinimal("signed integer")
                    }));
                }
                Ok(Value::Negative(value))
            }
            0xd9 => {
                let length = usize::from(self.byte()?);
                if length <= 31 {
                    return Err(self.error(ErrorKind::NonMinimal("text")));
                }
                self.text(length)
            }
            0xda => {
                let length = usize::from(self.u16()?);
                if length <= 255 {
                    return Err(self.error(ErrorKind::NonMinimal("text")));
                }
                self.text(length)
            }
            0xdb => {
                let length = usize::try_from(self.u32()?)
                    .map_err(|_| self.error(ErrorKind::LengthOverflow))?;
                if length <= 65_535 {
                    return Err(self.error(ErrorKind::NonMinimal("text")));
                }
                self.text(length)
            }
            0xdc => {
                let length = usize::from(self.u16()?);
                if length == 0 {
                    return Err(self.error(ErrorKind::EmptyArray));
                }
                if length <= 15 {
                    return Err(self.error(ErrorKind::NonMinimal("array")));
                }
                // go-foks's canonical checker for signable objects rejects every
                // array16 whose length is <= 0x1f, even 16..=31 which have no
                // fixarray form. The general RPC codec must keep accepting them
                // (the signup argument is array16(16)).
                if self.signable && length <= 0x1f {
                    return Err(self.error(ErrorKind::NonCanonicalSignable("array16")));
                }
                self.array(length)
            }
            0xdd => {
                let length = usize::try_from(self.u32()?)
                    .map_err(|_| self.error(ErrorKind::LengthOverflow))?;
                if length <= 65_535 {
                    return Err(self.error(ErrorKind::NonMinimal("array")));
                }
                self.array(length)
            }
            0xde..=0xdf => Err(self.error(ErrorKind::ArbitraryMap)),
            0xe0..=0xff => Ok(Value::Negative(i64::from(marker as i8))),
        }
    }

    fn signed(&self, value: i64, marker_minimum: i64) -> Result<Value, Error> {
        if value >= 0 {
            return Err(self.error(ErrorKind::NonNegativeSignedInteger));
        }
        let shorter_minimum = match marker_minimum {
            value if value == i64::from(i8::MIN) => -32,
            value if value == i64::from(i16::MIN) => i64::from(i8::MIN),
            value if value == i64::from(i32::MIN) => i64::from(i16::MIN),
            _ => unreachable!("signed called only for int8/int16/int32"),
        };
        if value >= shorter_minimum {
            return Err(self.error(ErrorKind::NonMinimal("signed integer")));
        }
        Ok(Value::Negative(value))
    }

    fn binary(&mut self, length: usize) -> Result<Value, Error> {
        self.bytes(length).map(Value::Binary)
    }

    fn text(&mut self, length: usize) -> Result<Value, Error> {
        self.bytes(length).map(Value::Text)
    }

    fn array(&mut self, length: usize) -> Result<Value, Error> {
        if self.input.len().saturating_sub(self.offset) < length {
            return Err(self.error(ErrorKind::UnexpectedEof));
        }
        if self.values.saturating_add(length) > MAXIMUM_DECODED_VALUES {
            return Err(self.error(ErrorKind::ValueLimit));
        }
        let mut values = Vec::with_capacity(length);
        for index in 0..length {
            self.path.push(PathSegment::Index(index));
            let result = self.value();
            self.path.pop();
            values.push(result?);
        }
        Ok(Value::Array(values))
    }

    fn variant(&mut self) -> Result<Value, Error> {
        let marker = self.byte()?;
        if !(0xa0..=0xbf).contains(&marker) {
            return Err(self.error(ErrorKind::InvalidVariantTag));
        }
        let tag = self.bytes((marker & 0x1f) as usize)?;
        self.path.push(PathSegment::Variant(tag.clone()));
        let result = self.value();
        self.path.pop();
        Ok(Value::Variant(Some((tag, Box::new(result?)))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_disallowed_marker_class_is_rejected() {
        for marker in [
            0xc1, 0xc7, 0xc8, 0xc9, 0xca, 0xcb, 0xd4, 0xd5, 0xd6, 0xd7, 0xd8, 0xde, 0xdf,
        ] {
            assert!(decode(&[marker]).is_err(), "marker {marker:#x}");
        }
        for marker in 0x82..=0x8f {
            assert!(matches!(
                decode(&[marker]).unwrap_err().kind,
                ErrorKind::InvalidVariantSize(_)
            ));
        }
    }

    #[test]
    fn nonminimal_integer_forms_are_rejected() {
        let cases: &[&[u8]] = &[
            &[0xcc, 0x7f],
            &[0xcd, 0x00, 0xff],
            &[0xce, 0x00, 0x00, 0xff, 0xff],
            &[0xcf, 0, 0, 0, 0, 0xff, 0xff, 0xff, 0xff],
            &[0xd0, 0xff],
            &[0xd1, 0xff, 0x80],
            &[0xd2, 0xff, 0xff, 0x80, 0x00],
            &[0xd3, 0xff, 0xff, 0xff, 0xff, 0x80, 0, 0, 0],
        ];
        for bytes in cases {
            assert!(decode(bytes).is_err(), "{bytes:02x?}");
        }
    }

    #[test]
    fn errors_retain_the_structural_path() {
        let error = decode(&[0x91, 0x81, 0xa1, b'1', 0xca]).unwrap_err();
        assert_eq!(
            error.path,
            vec![PathSegment::Index(0), PathSegment::Variant(vec![b'1'])]
        );
        assert_eq!(error.kind, ErrorKind::UnsupportedFloat);
    }

    fn array16(length: usize) -> Vec<u8> {
        let mut bytes = vec![0xdc];
        bytes.extend_from_slice(&u16::try_from(length).unwrap().to_be_bytes());
        bytes.extend(std::iter::repeat_n(0xc0, length));
        bytes
    }

    #[test]
    fn signable_validation_rejects_array16_up_to_31_but_general_decode_accepts_it() {
        // go-foks signs, verifies, and hashes over canonical bytes that reject
        // every array16 with 16..=31 elements, while the general RPC codec must
        // keep accepting them (the 16-field signup argument is array16(16)).
        for length in [16usize, 17, 31] {
            let bytes = array16(length);
            assert!(
                validate(&bytes).is_ok(),
                "general decode rejects len {length}"
            );
            assert_eq!(
                validate_signable(&bytes).unwrap_err().kind,
                ErrorKind::NonCanonicalSignable("array16"),
                "signable accepts len {length}"
            );
        }
        // 32 and above have no shorter form and are canonical for both.
        for length in [32usize, 40] {
            let bytes = array16(length);
            assert!(validate(&bytes).is_ok());
            assert!(
                validate_signable(&bytes).is_ok(),
                "signable rejects len {length}"
            );
        }
        // A fixarray is canonical under both.
        let fixarray = vec![0x93, 0xc0, 0xc0, 0xc0];
        assert!(validate(&fixarray).is_ok());
        assert!(validate_signable(&fixarray).is_ok());
    }

    #[test]
    fn signable_validation_recurses_into_nested_array16() {
        let mut nested = vec![0x91];
        nested.extend(array16(16));
        assert!(validate(&nested).is_ok());
        assert_eq!(
            validate_signable(&nested).unwrap_err().kind,
            ErrorKind::NonCanonicalSignable("array16")
        );
    }

    #[test]
    fn signable_validation_requires_full_consumption() {
        let mut trailing = vec![0x93, 0xc0, 0xc0, 0xc0];
        trailing.push(0xc0);
        assert_eq!(
            validate_signable(&trailing).unwrap_err().kind,
            ErrorKind::TrailingBytes
        );
    }

    #[test]
    fn malicious_array_count_fails_before_allocation() {
        let error = decode(&[0xdd, 0xff, 0xff, 0xff, 0xff]).unwrap_err();
        assert_eq!(error.kind, ErrorKind::UnexpectedEof);
    }

    #[test]
    fn array_value_amplification_is_rejected_before_allocation() {
        let length = MAXIMUM_DECODED_VALUES + 1;
        let mut encoded = Vec::with_capacity(length + 5);
        encoded.push(0xdd);
        encoded.extend_from_slice(&u32::try_from(length).unwrap().to_be_bytes());
        encoded.resize(length + 5, 0xc0);
        let error = decode(&encoded).unwrap_err();
        assert_eq!(error.kind, ErrorKind::ValueLimit);
    }
}
