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
    #[error("failed to determine YubiKey retry-count update outcome")]
    RetryUpdateUnknown,
    #[error("failed to confirm PIN restoration after updating YubiKey retry counts")]
    RetryPinRestore,
    #[error("failed to confirm PUK restoration after updating YubiKey retry counts")]
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
