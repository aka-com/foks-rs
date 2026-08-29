//! FOKS-only YubiKey lifecycle support.
//!
//! The default build contains the deterministic mock provider used by the
//! client/server integration suite. The `hardware` feature enables the
//! macOS/Linux PIV provider. No API in this crate reads an AKA path or invokes
//! a network service.

#![forbid(unsafe_code)]

mod device;
mod error;
mod mock;
mod provider;

#[cfg(feature = "hardware")]
mod hardware;

pub use device::{ManagedYubiDevice, PinRetries, PreparedYubiDevice, YubiAdministrativeDevice};
pub use error::{Error, Result};
pub use mock::MockYubiProvider;
pub use provider::{
    CardId, ManagementKey, Pin, PivPolicy, SlotId, YubiDeviceLocator, YubiProvider,
};

#[cfg(feature = "hardware")]
pub use hardware::HardwareYubiProvider;

/// True when this binary was built with the native PC/SC provider.
pub const HARDWARE_AVAILABLE: bool = cfg!(feature = "hardware");
