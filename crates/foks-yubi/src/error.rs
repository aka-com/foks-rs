use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("no matching hardware key was found")]
    NotFound,
    #[error("the selected hardware key was removed")]
    Removed,
    #[error("hardware key PIN is required")]
    PinRequired,
    #[error("hardware key PIN was rejected; {remaining} attempt(s) remain")]
    PinRejected { remaining: u8 },
    #[error("hardware key PIN is blocked")]
    PinBlocked,
    #[error("hardware key locator does not match the card's serial, slot, or public key")]
    LocatorMismatch,
    #[error("hardware key slot {0:#04x} is not a supported retired-key slot")]
    UnsupportedSlot(u8),
    #[error("hardware key operation requires the optional hardware feature")]
    HardwareUnavailable,
    #[error("hardware key management policy rejected the operation: {0}")]
    Policy(&'static str),
    #[error("failed to determine hardware key retry-count update outcome")]
    RetryUpdateUnknown,
    #[error("failed to confirm PIN restoration after updating hardware key retry counts")]
    RetryPinRestore,
    #[error("failed to confirm PUK restoration after updating hardware key retry counts")]
    RetryPukRestore,
    #[error("hardware key provider failed: {0}")]
    Provider(String),
    #[error("FOKS cryptography failed: {0}")]
    Crypto(#[from] foks_crypto::Error),
    #[error("FOKS protocol failed: {0}")]
    Protocol(#[from] foks_proto::Error),
    #[error("OS randomness is unavailable")]
    Randomness,
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
