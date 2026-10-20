//! Strict private continuation decoding. This validates structure, not SQL authority.
use super::{
    Error, InitialCursor, ListBinding, ReadDependency, Result, BINDING_DOMAIN, MAX_BINDING_BYTES,
};
use crate::{
    authority::{Fence, Number},
    personal::PeerCertificate,
};
use serde::{
    de::{self, SeqAccess, Visitor},
    Deserialize, Deserializer,
};
use std::fmt;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Binding {
    domain: String,
    fence: Fence,
    revision: Number,
    dependency: Dependency,
    peer: PeerCertificate,
    namespace: Vec<u8>,
    parent: [u8; 16],
    route: String,
    cursor: Cursor,
    count: Number,
    load_small: bool,
    deadline: Number,
    clock_domain: [u8; 32],
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Dependency {
    incarnation: [u8; 32],
    revision: Number,
}

enum CursorKind {
    None,
    Mac,
    Time,
}
impl<'de> Deserialize<'de> for CursorKind {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        struct KindVisitor;
        impl Visitor<'_> for KindVisitor {
            type Value = CursorKind;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a cursor kind string")
            }
            fn visit_str<E: de::Error>(self, value: &str) -> std::result::Result<Self::Value, E> {
                match value {
                    "none" => Ok(CursorKind::None),
                    "mac" => Ok(CursorKind::Mac),
                    "time" => Ok(CursorKind::Time),
                    _ => Err(E::custom("unknown cursor kind")),
                }
            }
        }
        d.deserialize_str(KindVisitor)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    kind: CursorKind,
    #[serde(default)]
    value: Presence,
}

#[derive(Default)]
enum Presence {
    #[default]
    Missing,
    Value(CursorValue),
}
impl<'de> Deserialize<'de> for Presence {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        CursorValue::deserialize(d).map(Self::Value)
    }
}

enum CursorValue {
    Mac([u8; 32]),
    Time(Number),
}
impl<'de> Deserialize<'de> for CursorValue {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        struct ValueVisitor;
        impl<'de> Visitor<'de> for ValueVisitor {
            type Value = CursorValue;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a canonical decimal string or exactly 32 bytes")
            }
            fn visit_str<E: de::Error>(self, text: &str) -> std::result::Result<Self::Value, E> {
                let value: i64 = text.parse().map_err(E::custom)?;
                if text != value.to_string() {
                    return Err(E::custom("noncanonical integer"));
                }
                Ok(CursorValue::Time(Number(value)))
            }
            fn visit_seq<A: SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> std::result::Result<Self::Value, A::Error> {
                let mut value = [0; 32];
                for (index, byte) in value.iter_mut().enumerate() {
                    *byte = seq
                        .next_element()?
                        .ok_or_else(|| de::Error::invalid_length(index, &self))?;
                }
                if seq.next_element::<de::IgnoredAny>()?.is_some() {
                    return Err(de::Error::invalid_length(33, &self));
                }
                Ok(CursorValue::Mac(value))
            }
        }
        d.deserialize_any(ValueVisitor)
    }
}
impl Cursor {
    fn checked(self) -> Result<InitialCursor> {
        match (self.kind, self.value) {
            (CursorKind::None, Presence::Missing) => Ok(InitialCursor::None {}),
            (CursorKind::Mac, Presence::Value(CursorValue::Mac(value))) => {
                Ok(InitialCursor::Mac { value })
            }
            (CursorKind::Time, Presence::Value(CursorValue::Time(value))) if value.0 >= 0 => {
                Ok(InitialCursor::Time { value })
            }
            _ => Err(Error::InvalidBinding),
        }
    }
}

impl ListBinding {
    /// Decode a bounded private binding without applying public-request defaults.
    /// This does not prove its peer, fence, dependency or token against live state.
    pub fn decode_wire(bytes: &[u8]) -> Result<Self> {
        if bytes.is_empty() || bytes.len() > MAX_BINDING_BYTES {
            return Err(Error::Bounds);
        }
        // No generic maps or enum Content trees are built. All owned variable
        // payloads are disjoint portions of the bounded input: typed byte Vecs
        // and strings need at most 2x their input share for capacity growth.
        // A further 2x input covers serde's escaped-string scratch/growth and
        // transient conversion overlap. Fixed-width fields live in the DTO.
        // This is payload admission, not an allocator/RSS measurement. Canonical
        // serialization is charged independently by the existing constructor.
        let parse_bytes = bytes.len().checked_mul(4).ok_or(Error::Bounds)?;
        foks_snowpack::reserve_allocation(parse_bytes).map_err(|_| Error::Bounds)?;
        let binding: Binding = serde_json::from_slice(bytes).map_err(|_| Error::InvalidBinding)?;
        if binding.domain != BINDING_DOMAIN
            || binding.route != "personal_list"
            || !(1..=4096).contains(&binding.count.0)
            || binding.fence.activation.limits.blob_bytes as u64 > u64::from(u32::MAX)
            || binding.fence.activation.limits.dirent_bytes as u64 > u64::from(u32::MAX)
        {
            return Err(Error::InvalidBinding);
        }
        let revision =
            u64::try_from(binding.dependency.revision.0).map_err(|_| Error::InvalidBinding)?;
        let dependency = ReadDependency::new(binding.dependency.incarnation, revision)?;
        let namespace = binding
            .namespace
            .try_into()
            .map_err(|_| Error::InvalidBinding)?;
        Self::new(
            binding.fence,
            binding.revision,
            dependency,
            binding.peer,
            namespace,
            binding.parent,
            binding.cursor.checked()?,
            binding.count.0 as u64,
            binding.load_small,
            binding.deadline,
            binding.clock_domain,
        )
    }
}

#[cfg(test)]
mod tests;
