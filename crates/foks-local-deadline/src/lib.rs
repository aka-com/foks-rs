#![forbid(unsafe_code)]
//! Strict same-host deadline coordinates. No caller uses these controls until
//! the complete local IPC migration; these are never FOKS network fields.
use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("local request deadline exceeded")]
    Expired,
    #[error("local deadline clock domain changed or does not match the agent")]
    DomainMismatch,
    #[error("invalid or out-of-range local deadline coordinate")]
    Range,
    #[error("local deadline clock is unavailable: {0}")]
    Unavailable(String),
}
pub type Result<T> = std::result::Result<T, Error>;

/// JSON deliberately uses a canonical decimal string, never a floating value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Decimal(u64);
impl Serialize for Decimal {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0.to_string())
    }
}
impl<'de> Deserialize<'de> for Decimal {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        struct Visitor;
        impl serde::de::Visitor<'_> for Visitor {
            type Value = Decimal;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a canonical unsigned decimal string")
            }
            fn visit_str<E: serde::de::Error>(self, s: &str) -> std::result::Result<Decimal, E> {
                if s.is_empty()
                    || s.len() > 20
                    || (s.len() > 1 && s.starts_with('0'))
                    || !s.bytes().all(|b| b.is_ascii_digit())
                {
                    return Err(E::custom("noncanonical deadline integer"));
                }
                s.parse().map(Decimal).map_err(E::custom)
            }
        }
        deserializer.deserialize_str(Visitor)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
struct BootId(String);
impl BootId {
    fn parse(s: &str) -> Result<Self> {
        if s.len() != 36
            || !s.bytes().enumerate().all(|(i, b)| {
                if [8, 13, 18, 23].contains(&i) {
                    b == b'-'
                } else {
                    b.is_ascii_digit() || (b'a'..=b'f').contains(&b)
                }
            })
        {
            return Err(Error::Range);
        }
        Ok(Self(s.to_owned()))
    }
}
impl<'de> Deserialize<'de> for BootId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        struct Visitor;
        impl serde::de::Visitor<'_> for Visitor {
            type Value = BootId;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a canonical lower-case boot UUID")
            }
            fn visit_str<E: serde::de::Error>(self, s: &str) -> std::result::Result<BootId, E> {
                BootId::parse(s).map_err(E::custom)
            }
        }
        deserializer.deserialize_str(Visitor)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "clock", deny_unknown_fields)]
enum DomainValue {
    #[serde(rename = "linux_monotonic_v1")]
    Linux {
        boot: BootId,
        device: Decimal,
        inode: Decimal,
    },
    #[serde(rename = "darwin_uptime_raw_v1")]
    Darwin { boot: BootId },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Domain(DomainValue);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Version;
impl Serialize for Version {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_u8(1)
    }
}
impl<'de> Deserialize<'de> for Version {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        if u8::deserialize(deserializer)? != 1 {
            return Err(serde::de::Error::custom("unsupported deadline version"));
        }
        Ok(Self)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeadlineEnvelope {
    version: Version,
    domain: Domain,
    expires_ns: Decimal,
}
impl DeadlineEnvelope {
    pub fn domain(&self) -> &Domain {
        &self.domain
    }
    pub fn expires_ns(&self) -> u64 {
        self.expires_ns.0
    }
}

#[cfg(feature = "clock")]
mod clock;
#[cfg(feature = "clock")]
pub use clock::{sample, ClockSample, Deadline};

#[cfg(test)]
mod tests;
