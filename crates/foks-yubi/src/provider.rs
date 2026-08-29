use std::fmt;

use serde::{Deserialize, Serialize};
use zeroize::Zeroize as _;

use crate::{ManagedYubiDevice, PreparedYubiDevice, Result, YubiAdministrativeDevice};

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct CardId {
    pub name: String,
    pub serial: u32,
}

/// FOKS v0.1.9 uses PIV retired-key slots 0x82 through 0x95.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct SlotId(u8);

impl SlotId {
    pub const MIN: u8 = 0x82;
    pub const MAX: u8 = 0x95;

    pub fn new(value: u8) -> crate::Result<Self> {
        if !(Self::MIN..=Self::MAX).contains(&value) {
            return Err(crate::Error::UnsupportedSlot(value));
        }
        Ok(Self(value))
    }

    pub const fn get(self) -> u8 {
        self.0
    }
}

impl<'de> Deserialize<'de> for SlotId {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = u8::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct YubiDeviceLocator {
    pub card: CardId,
    pub signing_slot: SlotId,
    pub pq_slot: SlotId,
    /// Compressed SEC1 P-256 key in the signing slot.
    #[serde(with = "array33")]
    pub signing_public_key: [u8; 33],
    /// Compressed SEC1 P-256 key used to derive the v0.1.9 ML-KEM seed.
    #[serde(with = "array33")]
    pub pq_public_key: [u8; 33],
    /// v0.1.9 `YubiPQKeyID` derived from the PQ slot's compressed key.
    pub pq_key_id: [u8; 32],
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PivPolicy {
    Never,
    Once,
    Always,
}

/// PIN text is deliberately non-cloneable and always zeroized on drop.
pub struct Pin(String);

impl Drop for Pin {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

impl Pin {
    pub fn new(value: impl Into<String>) -> crate::Result<Self> {
        let value = value.into();
        if !(6..=8).contains(&value.len()) || !value.bytes().all(|byte| byte.is_ascii_graphic()) {
            return Err(crate::Error::Policy(
                "PIN or PUK must contain six to eight printable ASCII characters",
            ));
        }
        Ok(Self(value))
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Pin {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Pin(<redacted>)")
    }
}

/// Optional retry policy applied only while preparing an unenrolled card.
/// The PUK is retained solely long enough to restore it after the PIV retry
/// command resets both credentials to their factory values.
pub struct PinRetryConfiguration {
    pub(crate) puk: Pin,
    pub(crate) pin_attempts: u8,
    pub(crate) puk_attempts: u8,
}

impl PinRetryConfiguration {
    pub fn new(puk: Pin, pin_attempts: u8, puk_attempts: u8) -> Result<Self> {
        if !(1..=15).contains(&pin_attempts) || !(1..=15).contains(&puk_attempts) {
            return Err(crate::Error::Policy(
                "PIN/PUK retries must be between one and fifteen",
            ));
        }
        Ok(Self {
            puk,
            pin_attempts,
            puk_attempts,
        })
    }
}

impl fmt::Debug for PinRetryConfiguration {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PinRetryConfiguration")
            .field("puk", &"<redacted>")
            .field("pin_attempts", &self.pin_attempts)
            .field("puk_attempts", &self.puk_attempts)
            .finish()
    }
}

pub struct ManagementKey([u8; 24]);

impl Drop for ManagementKey {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

impl ManagementKey {
    pub fn default_piv() -> Self {
        Self([
            1, 2, 3, 4, 5, 6, 7, 8, 1, 2, 3, 4, 5, 6, 7, 8, 1, 2, 3, 4, 5, 6, 7, 8,
        ])
    }

    pub fn random() -> crate::Result<Self> {
        let mut value = [0u8; 24];
        getrandom::fill(&mut value).map_err(|_| crate::Error::Randomness)?;
        Ok(Self(value))
    }

    pub fn from_bytes(value: [u8; 24]) -> Self {
        Self(value)
    }

    pub fn expose(&self) -> &[u8; 24] {
        &self.0
    }
}

impl fmt::Debug for ManagementKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ManagementKey(<redacted>)")
    }
}

pub trait YubiProvider: Send + Sync {
    fn cards(&self) -> Result<Vec<CardId>>;

    /// Generates two distinct P-256 keys. When retry configuration is
    /// requested, implementations must first prove that every PIV key slot is
    /// empty, apply and restore the PIN/PUK retry policy, and only then
    /// generate either key. The operation may partially change the card, so
    /// callers must durably record the returned public locator before
    /// beginning any server mutation.
    #[allow(clippy::too_many_arguments)]
    fn prepare(
        &self,
        card: &CardId,
        signing_slot: SlotId,
        pq_slot: SlotId,
        pin: &Pin,
        retry_configuration: Option<&PinRetryConfiguration>,
        pin_policy: PivPolicy,
        touch_policy: PivPolicy,
    ) -> Result<PreparedYubiDevice>;

    /// Opens an exact card/slot/public-key tuple and revalidates every field.
    fn open(
        &self,
        locator: &YubiDeviceLocator,
        pin: Option<&Pin>,
    ) -> Result<Box<dyn ManagedYubiDevice>>;

    /// Opens only the administrative PIV surface. No PIN or private-key
    /// operation is required, allowing retry recovery after PIN lockout.
    fn open_admin(&self, locator: &YubiDeviceLocator) -> Result<Box<dyn YubiAdministrativeDevice>>;
}

mod array33 {
    use serde::{de::Error as _, Deserialize as _, Deserializer, Serializer};

    pub fn serialize<S>(value: &[u8; 33], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_bytes(value)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<[u8; 33], D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = Vec::<u8>::deserialize(deserializer)?;
        value
            .try_into()
            .map_err(|value: Vec<u8>| D::Error::invalid_length(value.len(), &"33 bytes"))
    }
}
