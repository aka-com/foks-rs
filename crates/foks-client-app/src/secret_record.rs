//! Secret byte arrays and encoding for JSON records kept in the credential
//! vault.
//!
//! Vault records store seeds, removal keys and self tokens as JSON arrays of
//! numbers, the form serde produces for `[u8; N]`. `SecretArray` keeps that
//! encoding while holding the bytes in zeroizing storage and redacting them
//! from `Debug`, so a record needs neither a hand-written `Drop` nor a
//! hand-written `Debug` to protect its secrets.
use std::fmt;
use std::io;
use std::marker::PhantomData;

use foks_proto::SecretSeed;
use serde::de::{self, SeqAccess, Visitor};
use serde::ser::SerializeTuple;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use zeroize::{ZeroizeOnDrop, Zeroizing};

use crate::{Error, Result};

/// An `N`-byte secret that is zeroized on drop, redacted from `Debug`, and
/// serialized exactly as serde serializes `[u8; N]`.
pub(crate) struct SecretArray<const N: usize>(Zeroizing<[u8; N]>);

impl<const N: usize> SecretArray<N> {
    pub(crate) fn new(bytes: [u8; N]) -> Self {
        Self(Zeroizing::new(bytes))
    }

    /// Fills a new array from the operating system's random source without
    /// an intermediate ordinary array.
    pub(crate) fn random() -> Result<Self> {
        let mut secret = Self(Zeroizing::new([0u8; N]));
        getrandom::fill(secret.0.as_mut_slice()).map_err(|_| Error::Randomness)?;
        Ok(secret)
    }

    pub(crate) fn as_bytes(&self) -> &[u8; N] {
        &self.0
    }

    pub(crate) fn as_mut_bytes(&mut self) -> &mut [u8; N] {
        &mut self.0
    }
}

impl SecretArray<32> {
    /// Copies a seed into a record field without an intermediate ordinary
    /// array.
    pub(crate) fn from_seed(seed: &SecretSeed) -> Self {
        let mut secret = Self(Zeroizing::new([0u8; 32]));
        secret.0.copy_from_slice(seed.as_slice());
        secret
    }

    pub(crate) fn seed(&self) -> SecretSeed {
        SecretSeed::new(*self.0)
    }
}

impl<const N: usize> ZeroizeOnDrop for SecretArray<N> {}

impl<const N: usize> fmt::Debug for SecretArray<N> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[REDACTED]")
    }
}

impl<const N: usize> Serialize for SecretArray<N> {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        // serde's own `[u8; N]` impls only cover N <= 32 and encode an array
        // as a tuple of N elements; this emits the same tuple.
        let mut tuple = serializer.serialize_tuple(N)?;
        for byte in self.0.iter() {
            tuple.serialize_element(byte)?;
        }
        tuple.end()
    }
}

impl<'de, const N: usize> Deserialize<'de> for SecretArray<N> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        struct ArrayVisitor<const N: usize>(PhantomData<[u8; N]>);

        impl<'de, const N: usize> Visitor<'de> for ArrayVisitor<N> {
            type Value = SecretArray<N>;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(formatter, "an array of {N} bytes")
            }

            fn visit_seq<A: SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> std::result::Result<Self::Value, A::Error> {
                let mut secret = SecretArray(Zeroizing::new([0u8; N]));
                for (index, slot) in secret.0.iter_mut().enumerate() {
                    *slot = seq
                        .next_element()?
                        .ok_or_else(|| de::Error::invalid_length(index, &self))?;
                }
                Ok(secret)
            }
        }

        deserializer.deserialize_tuple(N, ArrayVisitor::<N>(PhantomData))
    }
}

/// Serializes `value` as JSON into a zeroizing buffer allocated at its final
/// size.
///
/// The value is serialized twice: once to measure the output and once into a
/// buffer of exactly that capacity. A buffer that grows while it is written
/// would leave earlier, unzeroed copies of the plaintext in freed allocations.
pub(crate) fn encode_private_json<T: Serialize + ?Sized>(
    value: &T,
) -> serde_json::Result<Zeroizing<Vec<u8>>> {
    let mut counter = ByteCounter(0);
    serde_json::to_writer(&mut counter, value)?;
    let mut encoded = Zeroizing::new(Vec::with_capacity(counter.0));
    serde_json::to_writer(&mut *encoded, value)?;
    debug_assert_eq!(encoded.len(), counter.0);
    Ok(encoded)
}

struct ByteCounter(usize);

impl io::Write for ByteCounter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0 += bytes.len();
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Formats `len` copies of `byte` the way serde_json encodes `[u8; len]`.
#[cfg(test)]
pub(crate) fn json_bytes(byte: u8, len: usize) -> String {
    format!("[{}]", vec![byte.to_string(); len].join(","))
}

/// Decodes a vault record from `json` and checks that both encoders
/// reproduce `json` byte for byte.
#[cfg(test)]
pub(crate) fn assert_json_round_trip<T: Serialize + serde::de::DeserializeOwned>(json: &str) -> T {
    let value: T = serde_json::from_str(json).unwrap();
    assert_eq!(
        String::from_utf8(serde_json::to_vec(&value).unwrap()).unwrap(),
        json
    );
    assert_eq!(
        encode_private_json(&value).unwrap().as_slice(),
        json.as_bytes()
    );
    value
}

/// Checks that a record whose secret bytes are all 0xab (171) does not print
/// them.
#[cfg(test)]
pub(crate) fn assert_debug_redacted(value: &impl fmt::Debug) {
    let debug = format!("{value:?} {value:#?}");
    assert!(debug.contains("[REDACTED]"), "{debug}");
    assert!(!debug.contains("171"), "{debug}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Serialize)]
    struct Plain {
        a: [u8; 16],
        b: [u8; 17],
        c: [u8; 32],
        d: Option<[u8; 32]>,
        e: Option<[u8; 32]>,
    }

    #[derive(Debug, Deserialize, Serialize)]
    struct Wrapped {
        a: SecretArray<16>,
        b: SecretArray<17>,
        c: SecretArray<32>,
        d: Option<SecretArray<32>>,
        e: Option<SecretArray<32>>,
    }

    fn counting<const N: usize>(start: u8) -> [u8; N] {
        std::array::from_fn(|index| start.wrapping_add(index as u8))
    }

    #[test]
    fn secret_array_json_matches_plain_array_json() {
        let plain = serde_json::to_vec(&Plain {
            a: counting(0),
            b: counting(100),
            c: counting(200),
            d: Some(counting(7)),
            e: None,
        })
        .unwrap();
        let wrapped: Wrapped = serde_json::from_slice(&plain).unwrap();
        assert_eq!(wrapped.a.as_bytes(), &counting::<16>(0));
        assert_eq!(wrapped.b.as_bytes(), &counting::<17>(100));
        assert_eq!(wrapped.c.as_bytes(), &counting::<32>(200));
        assert_eq!(wrapped.d.as_ref().unwrap().as_bytes(), &counting::<32>(7));
        assert!(wrapped.e.is_none());
        assert_eq!(serde_json::to_vec(&wrapped).unwrap(), plain);
        assert_eq!(*encode_private_json(&wrapped).unwrap(), plain);
    }

    #[test]
    fn secret_array_rejects_wrong_lengths_and_values() {
        assert!(serde_json::from_str::<SecretArray<4>>("[1,2,3,4]").is_ok());
        for input in [
            "[1,2,3]",
            "[1,2,3,4,5]",
            "[1,2,3,256]",
            "[1,2,3,-1]",
            "\"abcd\"",
        ] {
            assert!(
                serde_json::from_str::<SecretArray<4>>(input).is_err(),
                "{input} decoded"
            );
        }
    }

    #[test]
    fn secret_array_debug_is_redacted() {
        let wrapped = Wrapped {
            a: SecretArray::new([0xab; 16]),
            b: SecretArray::new([0xab; 17]),
            c: SecretArray::new([0xab; 32]),
            d: Some(SecretArray::new([0xab; 32])),
            e: None,
        };
        let debug = format!("{wrapped:?} {wrapped:#?}");
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("171"), "{debug}");
    }

    #[test]
    fn private_json_is_allocated_at_its_final_size() {
        let value = serde_json::json!({
            "text": "a string long enough to force several reallocations ".repeat(40),
            "bytes": vec![255u8; 300],
        });
        let encoded = encode_private_json(&value).unwrap();
        assert_eq!(*encoded, serde_json::to_vec(&value).unwrap());
        assert_eq!(encoded.capacity(), encoded.len());
    }

    #[test]
    fn random_secret_arrays_differ() {
        let first = SecretArray::<32>::random().unwrap();
        let second = SecretArray::<32>::random().unwrap();
        assert_ne!(first.as_bytes(), second.as_bytes());
    }
}
