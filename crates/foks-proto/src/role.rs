//! Exact protocol roles, including member visibility.

use crate::{Result, Value};

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(u8)]
pub enum RoleType {
    None = 0,
    Member = 1,
    Admin = 2,
    Owner = 3,
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Role {
    kind: RoleType,
    visibility: i16,
}

impl Role {
    pub const NONE: Self = Self::new_default(RoleType::None);
    pub const ADMIN: Self = Self::new_default(RoleType::Admin);
    pub const OWNER: Self = Self::new_default(RoleType::Owner);

    pub const fn member(visibility: i16) -> Self {
        Self {
            kind: RoleType::Member,
            visibility,
        }
    }

    pub const fn kind(self) -> RoleType {
        self.kind
    }

    pub const fn visibility(self) -> Option<i16> {
        match self.kind {
            RoleType::Member => Some(self.visibility),
            _ => None,
        }
    }

    pub const fn protocol_value(self) -> u64 {
        self.kind as u64
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        crate::identity::role(&crate::decode(bytes)?)
    }

    pub fn to_value(self) -> Value {
        let payload = match self.kind {
            RoleType::Member => Value::Variant(Some((
                b"0".to_vec(),
                Box::new(if self.visibility < 0 {
                    Value::Negative(i64::from(self.visibility))
                } else {
                    Value::Unsigned(self.visibility as u64)
                }),
            ))),
            _ => Value::Variant(None),
        };
        Value::Array(vec![Value::Unsigned(self.protocol_value()), payload])
    }

    const fn new_default(kind: RoleType) -> Self {
        Self {
            kind,
            visibility: 0,
        }
    }
}
