use foks_snowpack::{decode, encode, Value};

use crate::{array, boolean, option, type_error, unsigned, Result};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SemVer {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
}

impl SemVer {
    pub fn to_value(self) -> Value {
        Value::Array(vec![
            Value::Unsigned(self.major),
            Value::Unsigned(self.minor),
            Value::Unsigned(self.patch),
        ])
    }

    fn from_value(value: &Value) -> Result<Self> {
        let fields = array(value, 3)?;
        Ok(Self {
            major: unsigned(&fields[0])?,
            minor: unsigned(&fields[1])?,
            patch: unsigned(&fields[2])?,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClientVersionExt {
    pub version: SemVer,
    pub linker_version: Vec<u8>,
    pub linker_packaging: Vec<u8>,
}

impl ClientVersionExt {
    pub fn encoded(&self) -> Result<Vec<u8>> {
        Ok(encode(&Value::Array(vec![
            self.version.to_value(),
            Value::Text(self.linker_version.clone()),
            Value::Text(self.linker_packaging.clone()),
        ]))?)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let value = decode(bytes)?;
        let fields = array(&value, 3)?;
        let Value::Text(linker_version) = &fields[1] else {
            return Err(type_error("text", &fields[1]));
        };
        let Value::Text(linker_packaging) = &fields[2] else {
            return Err(type_error("text", &fields[2]));
        };
        Ok(Self {
            version: SemVer::from_value(&fields[0])?,
            linker_version: linker_version.clone(),
            linker_packaging: linker_packaging.clone(),
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServerClientVersionInfo {
    pub minimum: Option<SemVer>,
    pub newest: Option<SemVer>,
    pub message: Vec<u8>,
}

impl ServerClientVersionInfo {
    pub fn encoded(&self) -> Result<Vec<u8>> {
        Ok(encode(&Value::Array(vec![
            self.minimum.map_or(Value::Null, SemVer::to_value),
            self.newest.map_or(Value::Null, SemVer::to_value),
            Value::Text(self.message.clone()),
        ]))?)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let value = decode(bytes)?;
        let fields = array(&value, 3)?;
        let Value::Text(message) = &fields[2] else {
            return Err(type_error("text", &fields[2]));
        };
        Ok(Self {
            minimum: option(&fields[0], SemVer::from_value)?,
            newest: option(&fields[1], SemVer::from_value)?,
            message: message.clone(),
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DeviceNagInfo {
    pub num_devices: u64,
    pub cleared: bool,
}

impl DeviceNagInfo {
    pub fn encoded(self) -> Result<Vec<u8>> {
        Ok(encode(&Value::Array(vec![
            Value::Unsigned(self.num_devices),
            Value::Bool(self.cleared),
        ]))?)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let value = decode(bytes)?;
        let fields = array(&value, 2)?;
        Ok(Self {
            num_devices: unsigned(&fields[0])?,
            cleared: boolean(&fields[1])?,
        })
    }
}
