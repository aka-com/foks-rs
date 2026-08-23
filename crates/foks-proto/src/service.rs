//! Typed FOKS service identifiers.

use crate::{Error, Result};

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(u64)]
pub enum ServiceType {
    Registration = 1,
    User = 2,
    MerkleQuery = 5,
    Probe = 10,
    KvStore = 12,
    Realtime = 16,
}

impl ServiceType {
    pub const fn protocol_value(self) -> u64 {
        self as u64
    }
}

impl TryFrom<u64> for ServiceType {
    type Error = Error;

    fn try_from(value: u64) -> Result<Self> {
        match value {
            1 => Ok(Self::Registration),
            2 => Ok(Self::User),
            5 => Ok(Self::MerkleQuery),
            10 => Ok(Self::Probe),
            12 => Ok(Self::KvStore),
            16 => Ok(Self::Realtime),
            _ => Err(Error::UnknownEnum {
                kind: "service type",
                value,
            }),
        }
    }
}
