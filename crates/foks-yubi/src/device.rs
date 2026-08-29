use foks_crypto::YubiDevice;
use foks_proto::{EntityId, Hepk};

use crate::{ManagementKey, Pin, Result, YubiDeviceLocator};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PinRetries {
    pub remaining: u8,
    pub blocked: bool,
}

pub struct PreparedYubiDevice {
    pub locator: YubiDeviceLocator,
    pub entity_id: EntityId,
    pub hepk: Hepk,
    pub device: Box<dyn ManagedYubiDevice>,
}

/// Administrative PIV operations that remain available when the user PIN is
/// blocked. This deliberately does not require a FOKS signing/decapsulation
/// handle, since constructing one can itself require a valid PIN.
pub trait YubiAdministrativeDevice: Send + Sync {
    fn locator(&self) -> &YubiDeviceLocator;
    fn pin_retries(&self) -> Result<PinRetries>;
    /// Changes the PIN. Any separately opened cryptographic handle must be
    /// dropped and reopened with the new PIN after success.
    fn change_pin(&self, old: &Pin, new: &Pin) -> Result<()>;
    fn change_puk(&self, old: &Pin, new: &Pin) -> Result<()>;
    fn unblock_pin(&self, puk: &Pin, new_pin: &Pin) -> Result<()>;
    /// Changes both retry counters. PIV resets PIN and PUK to their factory
    /// values as part of this command, so implementations must verify first
    /// and restore `pin` and `puk` before returning success.
    fn set_pin_retries(
        &self,
        management_key: &ManagementKey,
        pin: &Pin,
        puk: &Pin,
        pin_attempts: u8,
        puk_attempts: u8,
    ) -> Result<()>;
    fn replace_management_key(
        &self,
        old: &ManagementKey,
        new: &ManagementKey,
        store_with_pin: bool,
    ) -> Result<()>;
}

pub trait ManagedYubiDevice: YubiDevice + YubiAdministrativeDevice {}
