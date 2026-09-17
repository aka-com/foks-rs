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
        let mut value = value.into();
        if !(6..=8).contains(&value.len()) || !value.bytes().all(|byte| byte.is_ascii_graphic()) {
            value.zeroize();
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

    pub fn puk(&self) -> &Pin {
        &self.puk
    }

    pub fn pin_attempts(&self) -> u8 {
        self.pin_attempts
    }

    pub fn puk_attempts(&self) -> u8 {
        self.puk_attempts
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

    /// Verifies the selected card, PIN, management key, and required empty
    /// slots without changing the card. `all_slots_empty` is required before a
    /// retry reset because that command can only be resumed safely before any
    /// enrollment key exists.
    fn prepare_preflight(
        &self,
        card: &CardId,
        signing_slot: SlotId,
        pq_slot: SlotId,
        pin: &Pin,
        all_slots_empty: bool,
    ) -> Result<()>;

    /// Applies PIV retry counts and leaves both credentials at their factory
    /// defaults. This operation is deliberately safe to repeat after an
    /// interrupted or ambiguously reported command.
    fn prepare_reset_retries(
        &self,
        card: &CardId,
        pin_attempts: u8,
        puk_attempts: u8,
    ) -> Result<()>;

    fn prepare_restore_pin(&self, card: &CardId, pin: &Pin) -> Result<()>;

    fn prepare_restore_puk(&self, card: &CardId, puk: &Pin) -> Result<()>;

    /// Generates a P-256 key in an empty preparation slot, or returns the
    /// public key already in that slot after a checkpointed generation was
    /// interrupted.
    fn prepare_key(
        &self,
        card: &CardId,
        slot: SlotId,
        pin_policy: PivPolicy,
        touch_policy: PivPolicy,
    ) -> Result<[u8; 33]>;

    /// Revalidates both generated keys and returns the usable device.
    fn finish_prepare(&self, locator: &YubiDeviceLocator, pin: &Pin) -> Result<PreparedYubiDevice>;

    /// Convenience preparation for callers that do not own durable storage.
    /// Product enrollment uses the individual methods above and checkpoints
    /// before and after every card mutation. Callers that request retry
    /// configuration must treat an error as potentially leaving both
    /// credentials at their factory defaults.
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
    ) -> Result<PreparedYubiDevice> {
        self.prepare_preflight(
            card,
            signing_slot,
            pq_slot,
            pin,
            retry_configuration.is_some(),
        )?;
        if let Some(retry) = retry_configuration {
            self.prepare_reset_retries(card, retry.pin_attempts, retry.puk_attempts)?;
            self.prepare_restore_pin(card, pin)?;
            self.prepare_restore_puk(card, &retry.puk)?;
        }
        let signing_public_key = self.prepare_key(card, signing_slot, pin_policy, touch_policy)?;
        let pq_public_key = self.prepare_key(card, pq_slot, pin_policy, touch_policy)?;
        let locator = YubiDeviceLocator {
            card: card.clone(),
            signing_slot,
            pq_slot,
            signing_public_key,
            pq_public_key,
            pq_key_id: foks_crypto::yubi_pq_key_id(&pq_public_key)?,
        };
        self.finish_prepare(&locator, pin)
    }

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
