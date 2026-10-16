//! Local upload identities. These never appear in the FOKS network protocol.
use std::{fmt, str::FromStr};

/// Keep the entire identity. Protected-record IDs are generated independently.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum UploadHandle {
    Legacy([u8; 16]),
    V1 {
        database_id: [u8; 16],
        issued_at: u64,
        random: [u8; 16],
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
#[error("upload handle must be canonical u1-<32 hex database ID>-<16 hex seconds>-<32 hex random>, or an existing legacy 32 hex ID")]
pub struct InvalidUploadHandle;

impl UploadHandle {
    pub const ENCODED_LEN: usize = 85;
    pub fn new(database_id: [u8; 16], issued_at: u64, random: [u8; 16]) -> Self {
        Self::V1 {
            database_id,
            issued_at,
            random,
        }
    }
    pub fn legacy(id: [u8; 16]) -> Self {
        Self::Legacy(id)
    }
    pub fn database_id(self) -> Option<[u8; 16]> {
        match self {
            Self::V1 { database_id, .. } => Some(database_id),
            Self::Legacy(_) => None,
        }
    }
    pub fn issued_at(self) -> Option<u64> {
        match self {
            Self::V1 { issued_at, .. } => Some(issued_at),
            Self::Legacy(_) => None,
        }
    }
    pub fn legacy_id(self) -> Option<[u8; 16]> {
        match self {
            Self::Legacy(id) => Some(id),
            _ => None,
        }
    }
    pub fn is_legacy(self) -> bool {
        matches!(self, Self::Legacy(_))
    }
}
fn hex(out: &mut fmt::Formatter<'_>, bytes: &[u8]) -> fmt::Result {
    for byte in bytes {
        write!(out, "{byte:02x}")?;
    }
    Ok(())
}
impl fmt::Display for UploadHandle {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Legacy(id) => hex(out, id),
            Self::V1 {
                database_id,
                issued_at,
                random,
            } => {
                out.write_str("u1-")?;
                hex(out, database_id)?;
                write!(out, "-{issued_at:016x}-")?;
                hex(out, random)
            }
        }
    }
}
fn parse_id(text: &str) -> Result<[u8; 16], InvalidUploadHandle> {
    if text.len() != 32 || !text.bytes().all(|b| matches!(b,b'0'..=b'9'|b'a'..=b'f')) {
        return Err(InvalidUploadHandle);
    }
    let mut bytes = [0; 16];
    for (i, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[i * 2..i * 2 + 2], 16).map_err(|_| InvalidUploadHandle)?;
    }
    Ok(bytes)
}
impl FromStr for UploadHandle {
    type Err = InvalidUploadHandle;
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        if text.len() == 32 {
            return Ok(Self::Legacy(parse_id(text)?));
        }
        let bytes = text.as_bytes();
        if bytes.len() != Self::ENCODED_LEN
            || !bytes.is_ascii()
            || &bytes[..3] != b"u1-"
            || bytes[35] != b'-'
            || bytes[52] != b'-'
            || !bytes[36..52]
                .iter()
                .all(|b| matches!(b,b'0'..=b'9'|b'a'..=b'f'))
        {
            return Err(InvalidUploadHandle);
        }
        Ok(Self::new(
            parse_id(&text[3..35])?,
            u64::from_str_radix(&text[36..52], 16).map_err(|_| InvalidUploadHandle)?,
            parse_id(&text[53..])?,
        ))
    }
}
impl serde::Serialize for UploadHandle {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}
impl<'de> serde::Deserialize<'de> for UploadHandle {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl serde::de::Visitor<'_> for Visitor {
            type Value = UploadHandle;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a canonical upload handle")
            }
            fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Self::Value, E> {
                value.parse().map_err(E::custom)
            }
        }
        deserializer.deserialize_str(Visitor)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn upload_identity_keeps_generation_time_and_all_random_bits() {
        for time in [0, u32::MAX as u64 + 1, u64::MAX] {
            let handle = UploadHandle::new([0x12; 16], time, [0xab; 16]);
            assert_eq!(handle.to_string().len(), 85);
            assert_eq!(handle.to_string().parse(), Ok(handle));
            assert!(handle
                .to_string()
                .parse::<crate::SubmissionHandle>()
                .is_err());
        }
        let legacy = UploadHandle::legacy([0xcd; 16]);
        assert_eq!(legacy.to_string().parse(), Ok(legacy));
    }
    #[test]
    fn malformed_or_other_domain_handles_are_rejected() {
        let valid = UploadHandle::new([0x12; 16], 7, [0xab; 16]).to_string();
        for text in [
            String::new(),
            valid.to_uppercase(),
            valid.replace("u1", "u2"),
            valid[1..].into(),
            format!("{valid}0"),
            valid.replace("ab", "é"),
            crate::SubmissionHandle::new(7, [0xab; 16]).to_string(),
        ] {
            assert!(text.parse::<UploadHandle>().is_err(), "{text}");
        }
    }
}
