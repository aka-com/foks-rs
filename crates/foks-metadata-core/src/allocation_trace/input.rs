use crate::{
    personal::{ListCursor, ListRequest, PeerCertificate},
    Error,
};
use serde::{
    de::{self, SeqAccess, Visitor},
    Deserialize, Deserializer,
};
use std::fmt;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    version: u8,
    peer: PeerCertificate,
    now_micros: Decimal,
    parent: [u8; 16],
    cursor: Cursor,
    count: Decimal,
    load_small: bool,
    frame: bool,
    rpc_sequence: Decimal,
    fail_at: Decimal,
}
struct Decimal(u64);
impl<'de> Deserialize<'de> for Decimal {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl Visitor<'_> for V {
            type Value = Decimal;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("canonical unsigned decimal")
            }
            fn visit_str<E: de::Error>(self, s: &str) -> Result<Decimal, E> {
                if s.is_empty()
                    || s.len() > 20
                    || s.starts_with('0') && s.len() > 1
                    || !s.bytes().all(|b| b.is_ascii_digit())
                {
                    return Err(E::custom("decimal"));
                }
                s.parse()
                    .map(Decimal)
                    .map_err(|_| E::custom("decimal range"))
            }
        }
        d.deserialize_str(V)
    }
}
enum Kind {
    Initial,
    Mac,
    Time,
}
impl<'de> Deserialize<'de> for Kind {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl Visitor<'_> for V {
            type Value = Kind;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("cursor kind")
            }
            fn visit_str<E: de::Error>(self, s: &str) -> Result<Kind, E> {
                match s {
                    "initial" => Ok(Kind::Initial),
                    "mac" => Ok(Kind::Mac),
                    "time" => Ok(Kind::Time),
                    _ => Err(E::custom("cursor kind")),
                }
            }
        }
        d.deserialize_str(V)
    }
}
#[derive(Default)]
enum Presence {
    #[default]
    Missing,
    Value(CursorValue),
}
enum CursorValue {
    Mac([u8; 32]),
    Time(u64),
}
impl<'de> Deserialize<'de> for Presence {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Presence;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("cursor value")
            }
            fn visit_str<E: de::Error>(self, s: &str) -> Result<Presence, E> {
                let n = Decimal::deserialize(serde::de::value::StrDeserializer::<E>::new(s))?;
                Ok(Presence::Value(CursorValue::Time(n.0)))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut a: A) -> Result<Presence, A::Error> {
                let mut b = [0; 32];
                for v in &mut b {
                    *v = a
                        .next_element()?
                        .ok_or_else(|| de::Error::custom("short cursor"))?;
                }
                if a.next_element::<de::IgnoredAny>()?.is_some() {
                    return Err(de::Error::custom("long cursor"));
                }
                Ok(Presence::Value(CursorValue::Mac(b)))
            }
        }
        d.deserialize_any(V)
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    kind: Kind,
    #[serde(default)]
    value: Presence,
}
pub struct Invocation {
    pub(super) peer: PeerCertificate,
    pub(super) now_micros: u64,
    pub(super) request: ListRequest,
    pub(super) frame: bool,
    pub(super) rpc_sequence: u64,
    pub(super) fail_at: u64,
}
impl Invocation {
    pub fn decode(bytes: &[u8]) -> crate::Result<Self> {
        if bytes.is_empty() || bytes.len() > super::MAX_INPUT {
            return Err(Error::Bounds);
        }
        let i: Input = serde_json::from_slice(bytes).map_err(|_| Error::Invalid("trace input"))?;
        if i.version != 1 || i.now_micros.0 > i64::MAX as u64 || i.fail_at.0 > 1_000_000 {
            return Err(Error::Invalid("trace input range"));
        }
        let cursor = match (i.cursor.kind, i.cursor.value) {
            (Kind::Initial, Presence::Missing) => ListCursor::Initial,
            (Kind::Mac, Presence::Value(CursorValue::Mac(v))) => ListCursor::Mac(v),
            (Kind::Time, Presence::Value(CursorValue::Time(v))) => ListCursor::Time(v),
            _ => return Err(Error::Invalid("trace cursor shape")),
        };
        Ok(Self {
            peer: i.peer,
            now_micros: i.now_micros.0,
            request: ListRequest::new(i.parent, cursor, i.count.0, i.load_small),
            frame: i.frame,
            rpc_sequence: i.rpc_sequence.0,
            fail_at: i.fail_at.0,
        })
    }
}
