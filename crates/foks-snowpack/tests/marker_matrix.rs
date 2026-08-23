use foks_snowpack::{decode, encode, validate, ErrorKind, Value};

fn repeated_null_array(marker: u8, length: usize) -> Vec<u8> {
    let mut bytes = vec![marker];
    bytes.extend(std::iter::repeat_n(0xc0, length));
    bytes
}

#[test]
fn every_messagepack_marker_has_an_explicit_policy() {
    for marker in 0_u8..=u8::MAX {
        let bytes = match marker {
            0x00..=0x7f | 0x80 | 0xc0 | 0xc2 | 0xc3 | 0xe0..=0xff => vec![marker],
            0x81 => vec![0x81, 0xa0, 0xc0],
            0x82..=0x8f | 0x90 | 0xc1 | 0xc7..=0xcb | 0xd4..=0xd8 | 0xde..=0xdf => {
                assert!(decode(&[marker]).is_err(), "marker {marker:#04x}");
                continue;
            }
            0x91..=0x9f => repeated_null_array(marker, usize::from(marker & 0x0f)),
            0xa0..=0xbf => {
                let mut bytes = vec![marker];
                bytes.resize(1 + usize::from(marker & 0x1f), 0);
                bytes
            }
            0xc4 => vec![0xc4, 0],
            0xc5 => {
                let mut bytes = vec![0xc5, 1, 0];
                bytes.resize(3 + 256, 0);
                bytes
            }
            0xc6 => {
                let mut bytes = vec![0xc6, 0, 1, 0, 0];
                bytes.resize(5 + 65_536, 0);
                bytes
            }
            0xcc => vec![0xcc, 0x80],
            0xcd => vec![0xcd, 1, 0],
            0xce => vec![0xce, 0, 1, 0, 0],
            0xcf => vec![0xcf, 0, 0, 0, 1, 0, 0, 0, 0],
            0xd0 => vec![0xd0, 0xdf],
            0xd1 => vec![0xd1, 0xff, 0x7f],
            0xd2 => vec![0xd2, 0xff, 0xff, 0x7f, 0xff],
            0xd3 => vec![0xd3, 0xff, 0xff, 0xff, 0xff, 0x7f, 0xff, 0xff, 0xff],
            0xd9 => {
                let mut bytes = vec![0xd9, 32];
                bytes.resize(2 + 32, 0);
                bytes
            }
            0xda => {
                let mut bytes = vec![0xda, 1, 0];
                bytes.resize(3 + 256, 0);
                bytes
            }
            0xdb => {
                let mut bytes = vec![0xdb, 0, 1, 0, 0];
                bytes.resize(5 + 65_536, 0);
                bytes
            }
            0xdc => {
                let mut bytes = vec![0xdc, 0, 32];
                bytes.extend(std::iter::repeat_n(0xc0, 32));
                bytes
            }
            0xdd => {
                let mut bytes = vec![0xdd, 0, 1, 0, 0];
                bytes.extend(std::iter::repeat_n(0xc0, 65_536));
                bytes
            }
        };

        validate(&bytes).unwrap_or_else(|error| panic!("marker {marker:#04x}: {error}"));
        let value = decode(&bytes).unwrap();
        assert_eq!(encode(&value).unwrap(), bytes, "marker {marker:#04x}");
    }
}

#[test]
fn signed_integer_markers_reject_nonnegative_payloads() {
    for bytes in [
        &[0xd0, 0][..],
        &[0xd1, 0, 0],
        &[0xd2, 0, 0, 0, 0],
        &[0xd3, 0, 0, 0, 0, 0, 0, 0, 0],
    ] {
        assert_eq!(
            decode(bytes).unwrap_err().kind,
            ErrorKind::NonNegativeSignedInteger
        );
    }
}

#[test]
fn scalar_value_variants_are_distinct() {
    assert_ne!(Value::Text(vec![]), Value::Binary(vec![]));
    assert_ne!(Value::Unsigned(0), Value::Negative(0));
}
