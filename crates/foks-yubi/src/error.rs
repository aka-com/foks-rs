use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("no matching YubiKey was found")]
    NotFound,
    #[error("the selected YubiKey was removed")]
    Removed,
    #[error("YubiKey PIN is required")]
    PinRequired,
    #[error("YubiKey PIN was rejected; {remaining} attempt(s) remain")]
    PinRejected { remaining: u8 },
    #[error("YubiKey PIN is blocked")]
    PinBlocked,
    #[error("YubiKey locator does not match the card's serial, slot, or public key")]
    LocatorMismatch,
    #[error("YubiKey slot {0:#04x} is not a supported retired-key slot")]
    UnsupportedSlot(u8),
    #[error("YubiKey operation requires the optional hardware feature")]
    HardwareUnavailable,
    #[error("YubiKey management policy rejected the operation: {0}")]
    Policy(&'static str),
    #[error(
        "YubiKey retry counts changed but the PIN could not be restored; PIN and PUK remain at their factory defaults"
    )]
    RetryPinRestore,
    #[error(
        "YubiKey retry counts and PIN changed but the PUK could not be restored; the PUK remains at its factory default"
    )]
    RetryPukRestore,
    #[error("YubiKey provider failed: {0}")]
    Provider(String),
    #[error("FOKS cryptography failed: {0}")]
    Crypto(#[from] foks_crypto::Error),
    #[error("FOKS protocol failed: {0}")]
    Protocol(#[from] foks_proto::Error),
    #[error("OS randomness is unavailable")]
    Randomness,
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
