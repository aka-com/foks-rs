use foks_proto::{BeaconHint, EntityId};
use foks_rpc::{decode_beacon_lookup_response, encode_beacon_lookup_request};

use crate::{FoksClient, ProbeTarget, Result};

/// A source of untrusted HostID-to-address routing hints.
pub trait BeaconResolver {
    fn lookup(&self, host: &EntityId) -> Result<BeaconHint>;
}

/// The v0.1.9 public Beacon RPC over WebPKI TLS.
pub struct NetworkBeacon<'a> {
    client: &'a FoksClient,
    target: ProbeTarget,
}

impl<'a> NetworkBeacon<'a> {
    pub fn new(client: &'a FoksClient, target: ProbeTarget) -> Self {
        Self { client, target }
    }
}

impl BeaconResolver for NetworkBeacon<'_> {
    fn lookup(&self, host: &EntityId) -> Result<BeaconHint> {
        let request = encode_beacon_lookup_request(host)?;
        let response = self.client.call_unpinned(&self.target, &request)?;
        decode_beacon_lookup_response(host.clone(), &response).map_err(Into::into)
    }
}
