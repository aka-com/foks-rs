//! Authenticated context for extended channel format 2, independent of Basic.
//! Sequence positions are allocated by the server and never inserted into this
//! client-authenticated context after encryption.
use crate::{
    array, expect_unsigned, fixed_blob, unsigned, Error, RealtimeWire, Result, Role,
    RoleAndGeneration, RtChannelId, RtHostId, RtTeamId, RtUserId, Value,
};

macro_rules! id {
    ($name:ident) => {
        #[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
        pub struct $name([u8; 16]);
        impl $name {
            pub fn new(bytes: [u8; 16]) -> Result<Self> {
                if bytes == [0; 16] {
                    return Err(Error::IntegerRange(stringify!($name)));
                }
                Ok(Self(bytes))
            }
            pub fn bytes(self) -> [u8; 16] {
                self.0
            }
        }
        impl RealtimeWire for $name {
            fn to_value(&self) -> Value {
                Value::Binary(self.0.to_vec())
            }
            fn from_value(value: &Value) -> Result<Self> {
                Self::new(fixed_blob(value, stringify!($name))?)
            }
        }
    };
}
id!(RtChatOperationId);
id!(RtChatEventId);

macro_rules! counter {
    ($name:ident) => {
        #[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Ord, PartialOrd)]
        pub struct $name(u64);
        impl $name {
            /// Zero is a cursor/baseline, never an inserted event or confirmation.
            pub fn new(value: u64) -> Result<Self> {
                if value > i64::MAX as u64 {
                    return Err(Error::IntegerRange(stringify!($name)));
                }
                Ok(Self(value))
            }
            pub fn get(self) -> u64 {
                self.0
            }
            pub fn next(self) -> Result<Self> {
                Self::new(self.0 + 1)
            }
        }
        impl RealtimeWire for $name {
            fn to_value(&self) -> Value {
                Value::Unsigned(self.0)
            }
            fn from_value(value: &Value) -> Result<Self> {
                Self::new(unsigned(value)?)
            }
        }
    };
}
counter!(RtChatEventPosition);
counter!(RtChatMessageOrdinal);
counter!(RtChatRevision);

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RtChatPurpose {
    Message {
        event: RtChatEventId,
    },
    Reply {
        event: RtChatEventId,
        target: RtChatEventId,
        root: RtChatEventId,
    },
    Edit {
        event: RtChatEventId,
        target: RtChatEventId,
    },
    Delete {
        event: RtChatEventId,
        target: RtChatEventId,
        moderation: bool,
    },
    Reaction {
        event: RtChatEventId,
        target: RtChatEventId,
        key: [u8; 32],
        present: bool,
    },
    Name {
        revision: RtChatRevision,
    },
    Description {
        revision: RtChatRevision,
    },
}

impl RealtimeWire for RtChatPurpose {
    fn validate(&self) -> Result<()> {
        match self {
            Self::Reply {
                event,
                target,
                root,
            } if event == target || event == root => {
                Err(Error::IntegerRange("self-referential reply"))
            }
            Self::Edit { event, target }
            | Self::Delete { event, target, .. }
            | Self::Reaction { event, target, .. }
                if event == target =>
            {
                Err(Error::IntegerRange("self-referential effect"))
            }
            Self::Reaction { key, .. } if *key == [0; 32] => {
                Err(Error::IntegerRange("empty reaction key"))
            }
            Self::Name { revision } | Self::Description { revision } if revision.get() == 0 => {
                Err(Error::IntegerRange("zero metadata revision"))
            }
            _ => Ok(()),
        }
    }
    fn to_value(&self) -> Value {
        use RtChatPurpose::*;
        let fields = match self {
            Message { event } => vec![Value::Unsigned(1), event.to_value()],
            Reply {
                event,
                target,
                root,
            } => vec![
                Value::Unsigned(2),
                event.to_value(),
                target.to_value(),
                root.to_value(),
            ],
            Edit { event, target } => vec![Value::Unsigned(3), event.to_value(), target.to_value()],
            Delete {
                event,
                target,
                moderation,
            } => vec![
                Value::Unsigned(4),
                event.to_value(),
                target.to_value(),
                moderation.to_value(),
            ],
            Reaction {
                event,
                target,
                key,
                present,
            } => vec![
                Value::Unsigned(5),
                event.to_value(),
                target.to_value(),
                Value::Binary(key.to_vec()),
                present.to_value(),
            ],
            Name { revision } => vec![Value::Unsigned(6), revision.to_value()],
            Description { revision } => vec![Value::Unsigned(7), revision.to_value()],
        };
        Value::Array(fields)
    }
    fn from_value(value: &Value) -> Result<Self> {
        let Value::Array(fields) = value else {
            return Err(Error::IntegerRange("chat purpose shape"));
        };
        let kind = fields
            .first()
            .ok_or(Error::IntegerRange("chat purpose kind"))
            .and_then(unsigned)?;
        let result = match kind {
            1 => {
                let f = array(value, 2)?;
                Self::Message {
                    event: RtChatEventId::from_value(&f[1])?,
                }
            }
            2 => {
                let f = array(value, 4)?;
                Self::Reply {
                    event: RtChatEventId::from_value(&f[1])?,
                    target: RtChatEventId::from_value(&f[2])?,
                    root: RtChatEventId::from_value(&f[3])?,
                }
            }
            3 => {
                let f = array(value, 3)?;
                Self::Edit {
                    event: RtChatEventId::from_value(&f[1])?,
                    target: RtChatEventId::from_value(&f[2])?,
                }
            }
            4 => {
                let f = array(value, 4)?;
                Self::Delete {
                    event: RtChatEventId::from_value(&f[1])?,
                    target: RtChatEventId::from_value(&f[2])?,
                    moderation: bool::from_value(&f[3])?,
                }
            }
            5 => {
                let f = array(value, 5)?;
                Self::Reaction {
                    event: RtChatEventId::from_value(&f[1])?,
                    target: RtChatEventId::from_value(&f[2])?,
                    key: fixed_blob(&f[3], "reaction key")?,
                    present: bool::from_value(&f[4])?,
                }
            }
            6 | 7 => {
                let f = array(value, 2)?;
                let revision = RtChatRevision::from_value(&f[1])?;
                if kind == 6 {
                    Self::Name { revision }
                } else {
                    Self::Description { revision }
                }
            }
            value => {
                return Err(Error::UnknownEnum {
                    kind: "chat encryption purpose",
                    value,
                })
            }
        };
        result.validate()?;
        Ok(result)
    }
}

/// All fields are public metadata. Target/root and reaction equality keys are
/// visible to the server. Message text and mention identities remain encrypted.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RtChatContext {
    pub host: RtHostId,
    pub team: RtTeamId,
    pub channel: RtChannelId,
    pub actor: RtUserId,
    pub operation: RtChatOperationId,
    pub key: RoleAndGeneration,
    pub purpose: RtChatPurpose,
}

impl RealtimeWire for RtChatContext {
    fn validate(&self) -> Result<()> {
        if self.channel.0 == [0; 16]
            || self.key.role == Role::NONE
            || self.key.generation == 0
            || self.key.generation > i64::MAX as u64
        {
            return Err(Error::IntegerRange("chat encryption context"));
        }
        self.purpose.validate()
    }
    fn to_value(&self) -> Value {
        Value::Array(vec![
            Value::Unsigned(2),
            self.host.to_value(),
            self.team.to_value(),
            self.channel.to_value(),
            self.actor.to_value(),
            self.operation.to_value(),
            self.key.to_value(),
            self.purpose.to_value(),
        ])
    }
    fn from_value(value: &Value) -> Result<Self> {
        let f = array(value, 8)?;
        expect_unsigned(&f[0], "extended chat format", 2)?;
        let result = Self {
            host: RtHostId::from_value(&f[1])?,
            team: RtTeamId::from_value(&f[2])?,
            channel: RtChannelId::from_value(&f[3])?,
            actor: RtUserId::from_value(&f[4])?,
            operation: RtChatOperationId::from_value(&f[5])?,
            key: RoleAndGeneration::from_value(&f[6])?,
            purpose: RtChatPurpose::from_value(&f[7])?,
        };
        result.validate()?;
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn extended_identities_and_cursors_reject_ambiguous_values() {
        assert!(RtChatOperationId::new([0; 16]).is_err());
        assert!(RtChatEventId::new([0; 16]).is_err());
        assert!(RtChatEventId::from_value(&Value::Binary(vec![1; 15])).is_err());
        assert_eq!(RtChatEventPosition::default().get(), 0);
        assert!(RtChatEventPosition::new(i64::MAX as u64)
            .unwrap()
            .next()
            .is_err());
        assert!(RtChatMessageOrdinal::new(u64::MAX).is_err());
        assert!(RtChatRevision::from_value(&Value::Negative(-1)).is_err());
        let large = RtChatMessageOrdinal::new((1 << 53) + 1).unwrap();
        assert_eq!(
            RtChatMessageOrdinal::decode(&large.encoded().unwrap()).unwrap(),
            large
        );
    }
    #[test]
    fn purpose_variants_have_exact_fields_and_valid_references() {
        let event = RtChatEventId::new([1; 16]).unwrap();
        let target = RtChatEventId::new([2; 16]).unwrap();
        let root = RtChatEventId::new([3; 16]).unwrap();
        for purpose in [
            RtChatPurpose::Message { event },
            RtChatPurpose::Reply {
                event,
                target,
                root,
            },
            RtChatPurpose::Edit { event, target },
            RtChatPurpose::Delete {
                event,
                target,
                moderation: true,
            },
            RtChatPurpose::Reaction {
                event,
                target,
                key: [4; 32],
                present: false,
            },
            RtChatPurpose::Name {
                revision: RtChatRevision::new(1).unwrap(),
            },
            RtChatPurpose::Description {
                revision: RtChatRevision::new(1).unwrap(),
            },
        ] {
            assert_eq!(
                RtChatPurpose::decode(&purpose.encoded().unwrap()).unwrap(),
                purpose
            );
            let Value::Array(mut fields) = purpose.to_value() else {
                unreachable!()
            };
            fields.push(Value::Null);
            assert!(RtChatPurpose::from_value(&Value::Array(fields)).is_err());
        }
        for purpose in [
            RtChatPurpose::Reply {
                event,
                target: event,
                root,
            },
            RtChatPurpose::Reply {
                event,
                target,
                root: event,
            },
            RtChatPurpose::Edit {
                event,
                target: event,
            },
            RtChatPurpose::Delete {
                event,
                target: event,
                moderation: false,
            },
            RtChatPurpose::Reaction {
                event,
                target,
                key: [0; 32],
                present: true,
            },
            RtChatPurpose::Name {
                revision: RtChatRevision::default(),
            },
        ] {
            assert!(purpose.encoded().is_err());
            assert!(RtChatPurpose::from_value(&purpose.to_value()).is_err());
        }
        assert!(RtChatPurpose::from_value(&Value::Array(vec![Value::Unsigned(99)])).is_err());
    }
}
