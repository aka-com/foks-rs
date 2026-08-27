use foks_proto::{DecodedProvisionDeviceArgument, DecodedRevokeDeviceArgument};

use crate::Result;

pub fn decode_provision_device(bytes: &[u8]) -> Result<DecodedProvisionDeviceArgument> {
    Ok(DecodedProvisionDeviceArgument::decode(bytes)?)
}

pub fn decode_revoke_device(bytes: &[u8]) -> Result<DecodedRevokeDeviceArgument> {
    Ok(DecodedRevokeDeviceArgument::decode(bytes)?)
}
