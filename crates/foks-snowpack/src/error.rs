use std::fmt;

/// Location within a positional Snowpack value.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PathSegment {
    Index(usize),
    Variant(Vec<u8>),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ErrorKind {
    UnexpectedEof,
    TrailingBytes,
    ReservedMarker(u8),
    UnsupportedFloat,
    UnsupportedExtension,
    ArbitraryMap,
    InvalidVariantSize(usize),
    InvalidVariantTag,
    EmptyArray,
    NonMinimal(&'static str),
    NonNegativeSignedInteger,
    LengthOverflow,
    DepthLimit,
}

/// A closed, offset-bearing Snowpack failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Error {
    pub kind: ErrorKind,
    pub offset: usize,
    pub path: Vec<PathSegment>,
}

impl Error {
    pub(crate) fn new(kind: ErrorKind, offset: usize, path: &[PathSegment]) -> Self {
        Self {
            kind,
            offset,
            path: path.to_vec(),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "Snowpack error at byte {}", self.offset)?;
        for segment in &self.path {
            match segment {
                PathSegment::Index(index) => write!(formatter, "[{index}]")?,
                PathSegment::Variant(tag) => {
                    write!(formatter, ".{}", String::from_utf8_lossy(tag))?
                }
            }
        }
        write!(formatter, ": ")?;
        match &self.kind {
            ErrorKind::UnexpectedEof => write!(formatter, "unexpected end of input"),
            ErrorKind::TrailingBytes => write!(formatter, "trailing bytes"),
            ErrorKind::ReservedMarker(marker) => {
                write!(formatter, "reserved marker {marker:#04x}")
            }
            ErrorKind::UnsupportedFloat => write!(formatter, "floats are not allowed"),
            ErrorKind::UnsupportedExtension => write!(formatter, "extensions are not allowed"),
            ErrorKind::ArbitraryMap => write!(formatter, "arbitrary maps are not allowed"),
            ErrorKind::InvalidVariantSize(size) => {
                write!(formatter, "variant map has {size} entries")
            }
            ErrorKind::InvalidVariantTag => write!(formatter, "invalid variant tag"),
            ErrorKind::EmptyArray => write!(formatter, "empty arrays must be null"),
            ErrorKind::NonMinimal(which) => write!(formatter, "non-minimal {which} encoding"),
            ErrorKind::NonNegativeSignedInteger => {
                write!(formatter, "signed integer encoding is not negative")
            }
            ErrorKind::LengthOverflow => write!(formatter, "length exceeds u32"),
            ErrorKind::DepthLimit => write!(formatter, "maximum nesting depth exceeded"),
        }
    }
}

impl std::error::Error for Error {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_error_kind_has_a_stable_human_readable_message() {
        let cases = [
            (ErrorKind::UnexpectedEof, "unexpected end of input"),
            (ErrorKind::TrailingBytes, "trailing bytes"),
            (ErrorKind::ReservedMarker(0xc1), "reserved marker 0xc1"),
            (ErrorKind::UnsupportedFloat, "floats are not allowed"),
            (
                ErrorKind::UnsupportedExtension,
                "extensions are not allowed",
            ),
            (ErrorKind::ArbitraryMap, "arbitrary maps are not allowed"),
            (
                ErrorKind::InvalidVariantSize(2),
                "variant map has 2 entries",
            ),
            (ErrorKind::InvalidVariantTag, "invalid variant tag"),
            (ErrorKind::EmptyArray, "empty arrays must be null"),
            (
                ErrorKind::NonMinimal("integer"),
                "non-minimal integer encoding",
            ),
            (
                ErrorKind::NonNegativeSignedInteger,
                "signed integer encoding is not negative",
            ),
            (ErrorKind::LengthOverflow, "length exceeds u32"),
            (ErrorKind::DepthLimit, "maximum nesting depth exceeded"),
        ];
        for (kind, expected) in cases {
            let error = Error::new(
                kind,
                7,
                &[PathSegment::Index(3), PathSegment::Variant(vec![0xff])],
            );
            assert_eq!(
                error.to_string(),
                format!("Snowpack error at byte 7[3].�: {expected}")
            );
        }
    }
}
