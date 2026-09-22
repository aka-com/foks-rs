//! Explicit FOKS Rust extensions. None of these values changes Go Basic RT schemas.
use crate::{array, expect_unsigned, Error, RealtimeWire, Result, RtHostId, Value};

/// Discovery is bound to the authenticated host, never a server version string.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RtChatCapabilitiesArgument {
    pub host: RtHostId,
}

impl RealtimeWire for RtChatCapabilitiesArgument {
    fn to_value(&self) -> Value {
        Value::Array(vec![Value::Unsigned(1), self.host.to_value()])
    }
    fn from_value(value: &Value) -> Result<Self> {
        let fields = array(value, 2)?;
        expect_unsigned(&fields[0], "chat capabilities request version", 1)?;
        Ok(Self {
            host: RtHostId::from_value(&fields[1])?,
        })
    }
}

/// Each bit represents implemented server behavior, not planned work. Version 1
/// describes extended channel format 2. Notifications are a local consumer and
/// have no server capability here. Unknown shapes/versions fail closed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RtChatCapabilities {
    pub host: RtHostId,
    pub extended_channels: bool,
    pub channel_management: bool,
    pub preferences: bool,
    pub content_actions: bool,
    pub threads: bool,
    pub attachments: bool,
}

impl RtChatCapabilities {
    pub fn basic_only(host: RtHostId) -> Self {
        Self {
            host,
            extended_channels: false,
            channel_management: false,
            preferences: false,
            content_actions: false,
            threads: false,
            attachments: false,
        }
    }
}

impl RealtimeWire for RtChatCapabilities {
    fn validate(&self) -> Result<()> {
        if (!self.extended_channels
            && (self.channel_management
                || self.preferences
                || self.content_actions
                || self.threads
                || self.attachments))
            || (!self.content_actions && (self.threads || self.attachments))
        {
            return Err(Error::IntegerRange("inconsistent chat capabilities"));
        }
        Ok(())
    }
    fn to_value(&self) -> Value {
        Value::Array(vec![
            Value::Unsigned(1),
            self.host.to_value(),
            self.extended_channels.to_value(),
            self.channel_management.to_value(),
            self.preferences.to_value(),
            self.content_actions.to_value(),
            self.threads.to_value(),
            self.attachments.to_value(),
        ])
    }
    fn from_value(value: &Value) -> Result<Self> {
        let fields = array(value, 8)?;
        expect_unsigned(&fields[0], "chat capabilities response version", 1)?;
        let result = Self {
            host: RtHostId::from_value(&fields[1])?,
            extended_channels: bool::from_value(&fields[2])?,
            channel_management: bool::from_value(&fields[3])?,
            preferences: bool::from_value(&fields[4])?,
            content_actions: bool::from_value(&fields[5])?,
            threads: bool::from_value(&fields[6])?,
            attachments: bool::from_value(&fields[7])?,
        };
        result.validate()?;
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn host() -> RtHostId {
        let mut bytes = vec![7; 33];
        bytes[0] = crate::ENTITY_HOST;
        RtHostId::new(crate::EntityId::from_bytes(bytes).unwrap()).unwrap()
    }
    #[test]
    fn capabilities_are_versioned_and_strict() {
        let request = RtChatCapabilitiesArgument { host: host() };
        assert_eq!(
            RtChatCapabilitiesArgument::decode(&request.encoded().unwrap()).unwrap(),
            request
        );
        let capabilities = RtChatCapabilities::basic_only(host());
        assert_eq!(
            RtChatCapabilities::decode(&capabilities.encoded().unwrap()).unwrap(),
            capabilities
        );
        let Value::Array(fields) = capabilities.to_value() else {
            unreachable!()
        };
        for value in [
            Value::Array(fields[..7].to_vec()),
            Value::Array({
                let mut v = fields.clone();
                v[0] = Value::Unsigned(2);
                v
            }),
            Value::Array({
                let mut v = fields.clone();
                v[2] = Value::Unsigned(1);
                v
            }),
            Value::Array({
                let mut v = fields;
                v.push(Value::Bool(false));
                v
            }),
        ] {
            assert!(RtChatCapabilities::from_value(&value).is_err());
        }
    }
    #[test]
    fn capabilities_do_not_claim_missing_dependencies() {
        let mut capabilities = RtChatCapabilities::basic_only(host());
        capabilities.threads = true;
        assert!(capabilities.encoded().is_err());
        assert!(RtChatCapabilities::from_value(&capabilities.to_value()).is_err());
        capabilities.extended_channels = true;
        assert!(capabilities.encoded().is_err());
        capabilities.content_actions = true;
        assert!(capabilities.encoded().is_ok());
    }
}
