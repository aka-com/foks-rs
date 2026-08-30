use foks_proto::ClientVersionExt;
use foks_snowpack::{decode, encode, Value};

use crate::{Error, Result};

pub fn decode_client_version_info(bytes: &[u8]) -> Result<ClientVersionExt> {
    let Value::Array(fields) = decode(bytes)? else {
        return Err(shape("one-field client-version argument"));
    };
    let [version] = fields.as_slice() else {
        return Err(shape("one-field client-version argument"));
    };
    Ok(ClientVersionExt::decode(&encode(version)?)?)
}

pub fn decode_clear_device_nag(bytes: &[u8]) -> Result<bool> {
    let Value::Array(fields) = decode(bytes)? else {
        return Err(shape("one-field clear-device-nag argument"));
    };
    let [Value::Bool(cleared)] = fields.as_slice() else {
        return Err(shape("clear-device-nag boolean"));
    };
    Ok(*cleared)
}

fn shape(expected: &'static str) -> Error {
    Error::Envelope {
        expected,
        found: "another Snowpack value",
    }
}
