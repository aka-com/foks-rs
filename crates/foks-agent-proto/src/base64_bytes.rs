//! Base64 serde adapters for the byte payloads this protocol carries.
//!
//! A JSON integer array costs `(10*1 + 90*2 + 156*3)/256 = 2.5703` digits plus
//! one separator for a uniformly distributed byte, so 3.5703 frame bytes per
//! content byte; ciphertext and compressed plaintext are uniform enough that
//! this is the rate in practice. Base64 costs a fixed `ceil(n/3)*4`, at most
//! 1.3334 plus two quotes. Every KV bound in this protocol is sized against
//! [`crate::MAXIMUM_MESSAGE_BYTES`] using that ratio, so the encoding and
//! those bounds are one decision and must change together, under one
//! [`crate::PROTOCOL_VERSION`] bump.
//!
//! The encoded string is the only intermediate either side materializes, and
//! both directions hold it in `Zeroizing`: an encoded chunk is as sensitive as
//! the plaintext it came from, so it must not be left in a freed allocation.
//! The frame buffer that carries it is the caller's to clear; the agent client
//! and [`crate::encode`] already keep frames in `Zeroizing`.
//!
//! Decoding is strict: the standard alphabet with canonical padding, so a
//! truncated or re-padded payload is a deserialization error rather than a
//! short byte string.

use base64::Engine as _;
use serde::{Deserialize as _, Deserializer, Serializer};
use zeroize::Zeroizing;

const ENGINE: base64::engine::general_purpose::GeneralPurpose =
    base64::engine::general_purpose::STANDARD;

pub fn serialize<S: Serializer>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
    let encoded = Zeroizing::new(ENGINE.encode(bytes));
    serializer.serialize_str(&encoded)
}

pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
    let encoded = Zeroizing::new(String::deserialize(deserializer)?);
    ENGINE
        .decode(encoded.as_bytes())
        .map_err(serde::de::Error::custom)
}

/// The same adapter for an absent-or-present payload. `None` stays JSON
/// `null`, which is what the `Option` fields on this protocol already mean.
pub mod optional {
    use super::{Deserializer, Serializer, Zeroizing, ENGINE};
    use base64::Engine as _;
    use serde::Deserialize as _;

    pub fn serialize<S: Serializer>(
        bytes: &Option<Vec<u8>>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        match bytes {
            Some(bytes) => {
                let encoded = Zeroizing::new(ENGINE.encode(bytes));
                serializer.serialize_some(encoded.as_str())
            }
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<Vec<u8>>, D::Error> {
        let encoded = Zeroizing::new(Option::<String>::deserialize(deserializer)?);
        match encoded.as_ref() {
            Some(encoded) => ENGINE
                .decode(encoded.as_bytes())
                .map(Some)
                .map_err(serde::de::Error::custom),
            None => Ok(None),
        }
    }
}

/// The frame cost of `length` plaintext bytes once base64-encoded and quoted,
/// used by the bounds that have to stay inside the frame ceiling.
pub fn encoded_frame_bytes(length: usize) -> usize {
    length.div_ceil(3) * 4 + 2
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
    struct Required {
        #[serde(with = "crate::base64_bytes")]
        content: Vec<u8>,
    }

    #[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
    struct Optional {
        #[serde(with = "crate::base64_bytes::optional")]
        content: Option<Vec<u8>>,
    }

    /// Byte values that stress the encoding: every byte value once, the
    /// three input lengths modulo the 3-byte group, an empty payload, and a
    /// run that decodes into padding characters.
    fn stressing_payloads() -> Vec<Vec<u8>> {
        let every_byte: Vec<u8> = (0u16..=255).map(|byte| byte as u8).collect();
        vec![
            Vec::new(),
            vec![0x00],
            vec![0xff],
            vec![0x00, 0xff],
            vec![0xfb, 0xff, 0xbf],
            b"====".to_vec(),
            every_byte.clone(),
            every_byte[..every_byte.len() - 1].to_vec(),
            every_byte[..every_byte.len() - 2].to_vec(),
            vec![0u8; 3 * 1024],
        ]
    }

    #[test]
    fn a_payload_round_trips_unchanged_through_the_adapter() {
        for payload in stressing_payloads() {
            let required = Required {
                content: payload.clone(),
            };
            let text = serde_json::to_string(&required).unwrap();
            assert_eq!(serde_json::from_str::<Required>(&text).unwrap(), required);
            // The wire form is one JSON string, not an integer array.
            assert!(text.starts_with(r#"{"content":""#), "{text}");

            let optional = Optional {
                content: Some(payload.clone()),
            };
            let text = serde_json::to_string(&optional).unwrap();
            assert_eq!(serde_json::from_str::<Optional>(&text).unwrap(), optional);
            // Values also survive the dynamic tree the desktop decodes from.
            assert_eq!(
                serde_json::from_value::<Optional>(serde_json::to_value(&optional).unwrap())
                    .unwrap(),
                optional
            );
        }
    }

    #[test]
    fn an_absent_payload_stays_absent() {
        let absent = Optional { content: None };
        let text = serde_json::to_string(&absent).unwrap();
        assert_eq!(text, r#"{"content":null}"#);
        assert_eq!(serde_json::from_str::<Optional>(&text).unwrap(), absent);
    }

    #[test]
    fn a_malformed_payload_is_an_error_rather_than_short_bytes() {
        // Truncated group, non-alphabet byte, and non-canonical padding each
        // fail instead of decoding to a prefix of the intended payload.
        for text in [
            r#"{"content":"AAAAA"}"#,
            r#"{"content":"AA!A"}"#,
            r#"{"content":"AA"}"#,
            r#"{"content":[1,2,3]}"#,
        ] {
            assert!(serde_json::from_str::<Required>(text).is_err(), "{text}");
        }
    }

    #[test]
    fn the_frame_cost_matches_what_the_encoder_produces() {
        for length in [0usize, 1, 2, 3, 4, 5, 6, 1000, 700 * 1024] {
            let payload = vec![0x5a; length];
            let text = serde_json::to_string(&payload_string(&payload)).unwrap();
            assert_eq!(text.len(), encoded_frame_bytes(length), "length {length}");
        }
    }

    fn payload_string(bytes: &[u8]) -> String {
        ENGINE.encode(bytes)
    }
}
