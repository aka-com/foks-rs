//! No derive-generated unknown-field or invalid-value diagnostics: they can
//! format untrusted strings before the caller discards the resulting error.
use super::{ContextError, HostListContext};
use crate::authority::{Activation, Fence, Identity, Number};
use crate::personal::PeerCertificate;
use crate::Limits;
use serde::de::{self, Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};
use std::fmt;

const MESSAGE: &str = "invalid host context";
fn invalid<E: de::Error>() -> E {
    E::custom(MESSAGE)
}

trait Shape<'de>: Sized {
    type Value;
    fn unsigned<E: de::Error>(self, _: u64) -> Result<Self::Value, E> {
        Err(invalid())
    }
    fn text<E: de::Error>(self, _: &'de str) -> Result<Self::Value, E> {
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
    fn visit_str<E: de::Error>(self, _: &str) -> Result<Self::Value, E> {
        Err(invalid())
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
        Err(invalid())
    }
    fn visit_some<D: Deserializer<'de>>(self, _: D) -> Result<Self::Value, D::Error> {
        Err(invalid())
    }
    fn visit_newtype_struct<D: Deserializer<'de>>(self, _: D) -> Result<Self::Value, D::Error> {
        Err(invalid())
    }
}

struct Field<'de>(&'de str);
impl<'de> Deserialize<'de> for Field<'de> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Text;
        impl<'de> Shape<'de> for Text {
            type Value = Field<'de>;
            fn text<E: de::Error>(self, text: &'de str) -> Result<Self::Value, E> {
                Ok(Field(text))
            }
        }
        deserializer.deserialize_any(Strict(Text))
    }
}

struct Byte(u8);
impl<'de> Deserialize<'de> for Byte {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Unsigned;
        impl<'de> Shape<'de> for Unsigned {
            type Value = Byte;
            fn unsigned<E: de::Error>(self, value: u64) -> Result<Self::Value, E> {
                Ok(Byte(value.try_into().map_err(|_| invalid::<E>())?))
            }
        }
        deserializer.deserialize_any(Strict(Unsigned))
    }
}

struct Decimal(u64);
impl<'de> Deserialize<'de> for Decimal {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Digits;
        impl<'de> Shape<'de> for Digits {
            type Value = Decimal;
            fn text<E: de::Error>(self, value: &'de str) -> Result<Self::Value, E> {
                if value.is_empty()
                    || value.len() > 20
                    || (value.len() > 1 && value.starts_with('0'))
                    || !value.bytes().all(|byte| byte.is_ascii_digit())
                {
                    return Err(invalid());
                }
                Ok(Decimal(value.parse().map_err(|_| invalid::<E>())?))
            }
        }
        deserializer.deserialize_any(Strict(Digits))
    }
}

struct Fixed<const N: usize>([u8; N]);
impl<'de, const N: usize> Deserialize<'de> for Fixed<N> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Bytes<const N: usize>;
        impl<'de, const N: usize> Shape<'de> for Bytes<N> {
            type Value = Fixed<N>;
            fn seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                let mut bytes = [0; N];
                for byte in &mut bytes {
                    *byte = seq
                        .next_element::<Byte>()?
                        .ok_or_else(invalid::<A::Error>)?
                        .0;
                }
                if seq.next_element::<Byte>()?.is_some() {
                    return Err(invalid());
                }
                Ok(Fixed(bytes))
            }
        }
        deserializer.deserialize_any(Strict(Bytes::<N>))
    }
}

struct Bounded<const N: usize>(Vec<u8>);
impl<'de, const N: usize> Deserialize<'de> for Bounded<N> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Bytes<const N: usize>;
        impl<'de, const N: usize> Shape<'de> for Bytes<N> {
            type Value = Bounded<N>;
            fn seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                let mut bytes = Vec::new();
                bytes
                    .try_reserve_exact(N)
                    .map_err(|_| invalid::<A::Error>())?;
                while let Some(Byte(byte)) = seq.next_element::<Byte>()? {
                    if bytes.len() == N {
                        return Err(invalid());
                    }
                    bytes.push(byte);
                }
                Ok(Bounded(bytes))
            }
        }
        deserializer.deserialize_any(Strict(Bytes::<N>))
    }
}

// Each object has fixed local slots; no map, String key, unknown-value parsing,
// or derived duplicate/unknown/invalid diagnostics are involved.
macro_rules! object {
    ($name:ident { $($field:ident: $kind:ty),+ $(,)? }) => {
        pub(super) struct $name { $($field: $kind),+ }
        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                struct Object;
                impl<'de> Shape<'de> for Object {
                    type Value = $name;
                    fn map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                        $(let mut $field = None;)+
                        while let Some(Field(field)) = map.next_key::<Field<'de>>()? {
                            match field {
                                $(stringify!($field) => {
                                    if $field.is_some() { return Err(invalid()); }
                                    $field = Some(map.next_value::<$kind>()?);
                                },)+
                                _ => return Err(invalid()),
                            }
                        }
                        Ok($name { $($field: $field.ok_or_else(invalid::<A::Error>)?),+ })
                    }
                }
                deserializer.deserialize_any(Strict(Object))
            }
        }
    };
}

object!(PeerParts {
    exact_der: Bounded<4096>, serial_magnitude: Bounded<20>,
    public_key: Fixed<32>, expires_at_micros: Decimal,
});
object!(IdentityParts {
    installation: Fixed<16>, tenant: Fixed<16>, host: Bounded<33>, cell: Fixed<32>,
});
object!(LimitsParts {
    blob_bytes: Decimal,
    dirent_bytes: Decimal,
    namespace_bytes: Decimal,
    namespace_objects: Decimal,
    directories: Decimal,
    dirents: Decimal,
});
object!(ActivationParts {
    version: Decimal, schema: Decimal, schema_hash: Fixed<32>, kernel_hash: Fixed<32>,
    catalog_hash: Fixed<32>, key_generation: Decimal, key_digest: Fixed<32>,
    storage_namespace: Fixed<16>, storage_placement: Fixed<32>, limits: LimitsParts,
});
object!(FenceParts {
    identity: IdentityParts, activation: ActivationParts, owner: Fixed<16>, epoch: Decimal,
});
object!(ContextParts {
    version: Byte,
    peer: PeerParts,
    fence: FenceParts,
    now_micros: Decimal,
    execution_deadline_ms: Decimal,
});

fn positive(value: Decimal) -> Result<Number, ContextError> {
    let value = i64::try_from(value.0).map_err(|_| ContextError::Invalid)?;
    if value == 0 {
        return Err(ContextError::Invalid);
    }
    Ok(Number(value))
}
fn allocation_limit(value: Decimal) -> Result<usize, ContextError> {
    let width = u32::try_from(value.0).map_err(|_| ContextError::Invalid)?;
    usize::try_from(width).map_err(|_| ContextError::Invalid)
}
impl ContextParts {
    pub(super) fn into_context(self) -> Result<HostListContext, ContextError> {
        if self.version.0 != 1 || self.fence.identity.host.0.len() != 33 {
            return Err(ContextError::Invalid);
        }
        let now_micros = self.now_micros.0;
        i64::try_from(now_micros).map_err(|_| ContextError::Invalid)?;
        let now_ms = (now_micros / 1000) as i64;
        let deadline = positive(self.execution_deadline_ms)?.0;
        if deadline
            .checked_sub(now_ms)
            .is_none_or(|delta| !(1..=15_000).contains(&delta))
        {
            return Err(ContextError::Invalid);
        }
        let p = self.peer;
        let peer = PeerCertificate::from_frontend_parts(
            p.exact_der.0,
            p.serial_magnitude.0,
            p.public_key.0,
            p.expires_at_micros.0,
        )
        .map_err(|_| ContextError::Invalid)?;
        let f = self.fence;
        let a = f.activation;
        let l = a.limits;
        Ok(HostListContext {
            peer,
            now_micros,
            execution_deadline_ms: deadline,
            fence: Fence {
                identity: Identity {
                    installation: f.identity.installation.0,
                    tenant: f.identity.tenant.0,
                    host: f.identity.host.0,
                    cell: f.identity.cell.0,
                },
                activation: Activation {
                    version: positive(a.version)?,
                    schema: positive(a.schema)?,
                    schema_hash: a.schema_hash.0,
                    kernel_hash: a.kernel_hash.0,
                    catalog_hash: a.catalog_hash.0,
                    key_generation: positive(a.key_generation)?,
                    key_digest: a.key_digest.0,
                    storage_namespace: a.storage_namespace.0,
                    storage_placement: a.storage_placement.0,
                    limits: Limits {
                        blob_bytes: allocation_limit(l.blob_bytes)?,
                        dirent_bytes: allocation_limit(l.dirent_bytes)?,
                        namespace_bytes: l.namespace_bytes.0,
                        namespace_objects: l.namespace_objects.0,
                        directories: l.directories.0,
                        dirents: l.dirents.0,
                    },
                },
                owner: f.owner.0,
                epoch: positive(f.epoch)?,
            },
        })
    }
}

const _: () = assert!(std::mem::size_of::<ContextParts>() <= 2048);
const _: () = assert!(MESSAGE.len() <= 64);

#[cfg(test)]
pub(super) fn vector_addresses(bytes: &[u8]) -> ([usize; 3], HostListContext) {
    let value: ContextParts = serde_json::from_slice(bytes).unwrap();
    let pointers = [
        value.peer.exact_der.0.as_ptr() as usize,
        value.peer.serial_magnitude.0.as_ptr() as usize,
        value.fence.identity.host.0.as_ptr() as usize,
    ];
    (pointers, value.into_context().unwrap())
}

#[cfg(test)]
pub(super) fn parser_error(bytes: &[u8]) -> Option<String> {
    // Diagnostics are inspected only outside the actual decoder/admission path.
    serde_json::from_slice::<ContextParts>(bytes)
        .err()
        .map(|error| error.to_string())
}

#[cfg(test)]
pub(super) fn rejected_capacity_message() -> String {
    // Capacity overflow exercises the same bounded visitor allocation-failure
    // branch without asking the allocator to consume unavailable memory.
    serde_json::from_slice::<Bounded<{ usize::MAX }>>(b"[]")
        .err()
        .unwrap()
        .to_string()
}
