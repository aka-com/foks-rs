//! Federated host discovery with an explicit untrusted-hint boundary.

mod beacon;

use std::path::Path;

use foks_client_db::SoftStateStore;
use foks_proto::{BeaconHint, EntityId, ENTITY_HOST};

use crate::{now_microseconds, Error, FoksClient, ProbeOutcome, ProbeTarget, Result};

pub use beacon::{BeaconResolver, NetworkBeacon};

#[derive(Debug)]
pub struct DiscoveryOutcome {
    pub hint: BeaconHint,
    pub probe: ProbeOutcome,
    /// Soft-state failure does not undo or invalidate an authenticated durable
    /// pin. Callers may report this warning and rebuild the cache later.
    pub cache_warning: Option<foks_client_db::Error>,
}

impl FoksClient {
    /// Resolves a HostID through a Beacon, then directly probes and durably
    /// pins the returned endpoint. Neither the Beacon nor the local cache is
    /// an authority for the host identity.
    pub fn discover_and_pin(
        &self,
        requested_host: &EntityId,
        beacon: &dyn BeaconResolver,
        hard_state_path: &Path,
        soft_state: Option<&mut SoftStateStore>,
    ) -> Result<DiscoveryOutcome> {
        requested_host.clone().require_type(ENTITY_HOST)?;
        let hint = beacon.lookup(requested_host)?;
        let target = validate_hint(requested_host, &hint)?;
        let probe = self.probe_and_pin_host_id(&target, requested_host, hard_state_path)?;
        let cache_warning = match soft_state {
            Some(store) => store.store_discovery_hint(&hint, now_microseconds()?).err(),
            None => None,
        };
        Ok(DiscoveryOutcome {
            hint,
            probe,
            cache_warning,
        })
    }
}

fn validate_hint(requested_host: &EntityId, hint: &BeaconHint) -> Result<ProbeTarget> {
    if &hint.host_id != requested_host {
        return Err(Error::FederationDiscovery(
            "Beacon returned an answer for a different HostID",
        ));
    }
    ProbeTarget::parse(&hint.address)
}

#[cfg(test)]
mod tests {
    use super::*;
    use foks_proto::ENTITY_USER;

    fn entity(kind: u8, fill: u8) -> EntityId {
        EntityId::from_bytes([vec![kind], vec![fill; 32]].concat()).unwrap()
    }

    #[test]
    fn beacon_hint_is_only_routing_for_the_exact_requested_host() {
        let requested = entity(ENTITY_HOST, 1);
        let other = entity(ENTITY_HOST, 2);
        let mismatched = BeaconHint::new(other, "example.test:4430".to_owned()).unwrap();
        assert!(matches!(
            validate_hint(&requested, &mismatched),
            Err(Error::FederationDiscovery(_))
        ));

        let valid = BeaconHint::new(requested.clone(), "Example.Test:4430".to_owned()).unwrap();
        assert_eq!(
            validate_hint(&requested, &valid).unwrap().address(),
            "example.test:4430"
        );
        assert!(validate_hint(&entity(ENTITY_USER, 3), &valid).is_err());
    }
}
