use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use foks_crypto::{HybridSecretDecapsulator, YubiDevice};
use foks_proto::{DhPublicKey, EntityId, Hepk};
use p256::ecdsa::{signature::hazmat::PrehashSigner as _, Signature as P256Signature, SigningKey};
use p256::elliptic_curve::{rand_core::OsRng, sec1::ToEncodedPoint as _};
use p256::{ecdh::diffie_hellman, PublicKey, SecretKey};
use zeroize::Zeroizing;

use crate::{
    CardId, Error, ManagedYubiDevice, ManagementKey, Pin, PinRetries, PinRetryConfiguration,
    PivPolicy, PreparedYubiDevice, Result, SlotId, YubiAdministrativeDevice, YubiDeviceLocator,
    YubiProvider,
};

const DEFAULT_MANAGEMENT_KEY: [u8; 24] = [
    1, 2, 3, 4, 5, 6, 7, 8, 1, 2, 3, 4, 5, 6, 7, 8, 1, 2, 3, 4, 5, 6, 7, 8,
];

#[derive(Clone)]
pub struct MockYubiProvider {
    cards: Arc<Mutex<BTreeMap<u32, Arc<Mutex<MockCard>>>>>,
}

struct MockCard {
    id: CardId,
    pin: String,
    puk: String,
    pin_attempts: u8,
    pin_remaining: u8,
    puk_attempts: u8,
    puk_remaining: u8,
    management_key: [u8; 24],
    slots: BTreeMap<SlotId, SecretKey>,
    #[cfg(test)]
    fail_retry_update_unknown: bool,
    #[cfg(test)]
    fail_retry_pin_restore: bool,
}

impl Drop for MockCard {
    fn drop(&mut self) {
        use zeroize::Zeroize as _;
        self.pin.zeroize();
        self.puk.zeroize();
        self.management_key.zeroize();
    }
}

impl MockYubiProvider {
    pub fn with_card(name: impl Into<String>, serial: u32, pin: &Pin) -> Result<Self> {
        if serial == 0 {
            return Err(Error::Policy("card serial must be nonzero"));
        }
        let card = MockCard {
            id: CardId {
                name: name.into(),
                serial,
            },
            pin: pin.expose().to_owned(),
            puk: "12345678".to_owned(),
            pin_attempts: 3,
            pin_remaining: 3,
            puk_attempts: 3,
            puk_remaining: 3,
            management_key: DEFAULT_MANAGEMENT_KEY,
            slots: BTreeMap::new(),
            #[cfg(test)]
            fail_retry_update_unknown: false,
            #[cfg(test)]
            fail_retry_pin_restore: false,
        };
        Ok(Self {
            cards: Arc::new(Mutex::new(BTreeMap::from([(
                serial,
                Arc::new(Mutex::new(card)),
            )]))),
        })
    }

    fn card(&self, id: &CardId) -> Result<Arc<Mutex<MockCard>>> {
        let cards = self
            .cards
            .lock()
            .map_err(|_| Error::Provider("mock card lock poisoned".into()))?;
        let card = cards.get(&id.serial).cloned().ok_or(Error::NotFound)?;
        if card
            .lock()
            .map_err(|_| Error::Provider("mock card lock poisoned".into()))?
            .id
            != *id
        {
            return Err(Error::LocatorMismatch);
        }
        Ok(card)
    }
}

impl YubiProvider for MockYubiProvider {
    fn cards(&self) -> Result<Vec<CardId>> {
        let cards = self
            .cards
            .lock()
            .map_err(|_| Error::Provider("mock card lock poisoned".into()))?;
        cards
            .values()
            .map(|card| {
                card.lock()
                    .map(|card| card.id.clone())
                    .map_err(|_| Error::Provider("mock card lock poisoned".into()))
            })
            .collect()
    }

    #[allow(clippy::too_many_arguments)]
    fn prepare(
        &self,
        id: &CardId,
        signing_slot: SlotId,
        pq_slot: SlotId,
        pin: &Pin,
        retry_configuration: Option<&PinRetryConfiguration>,
        _pin_policy: PivPolicy,
        _touch_policy: PivPolicy,
    ) -> Result<PreparedYubiDevice> {
        if signing_slot == pq_slot {
            return Err(Error::Policy("signing and PQ slots must be distinct"));
        }
        let card = self.card(id)?;
        {
            let mut card = card
                .lock()
                .map_err(|_| Error::Provider("mock card lock poisoned".into()))?;
            verify_pin(&mut card, pin)?;
            if card.slots.contains_key(&signing_slot) || card.slots.contains_key(&pq_slot) {
                return Err(Error::Policy("selected PIV slot is not empty"));
            }
            if let Some(retry) = retry_configuration {
                if !card.slots.is_empty() {
                    return Err(Error::Policy(
                        "PIV retry configuration requires every key slot to be empty",
                    ));
                }
                configure_pin_retries(
                    &mut card,
                    &ManagementKey::default_piv(),
                    pin,
                    &retry.puk,
                    retry.pin_attempts,
                    retry.puk_attempts,
                )?;
            }
            card.slots
                .insert(signing_slot, SecretKey::random(&mut OsRng));
            card.slots.insert(pq_slot, SecretKey::random(&mut OsRng));
        }
        let locator = locator_from_card(&card, signing_slot, pq_slot)?;
        let device = MockYubiDevice::new(card, locator.clone(), true)?;
        Ok(PreparedYubiDevice {
            locator,
            entity_id: device.id.clone(),
            hepk: device.hepk.clone(),
            device: Box::new(device),
        })
    }

    fn open(
        &self,
        locator: &YubiDeviceLocator,
        pin: Option<&Pin>,
    ) -> Result<Box<dyn ManagedYubiDevice>> {
        let card = self.card(&locator.card)?;
        let actual = locator_from_card(&card, locator.signing_slot, locator.pq_slot)?;
        if &actual != locator {
            return Err(Error::LocatorMismatch);
        }
        let authorized = if let Some(pin) = pin {
            let mut guard = card
                .lock()
                .map_err(|_| Error::Provider("mock card lock poisoned".into()))?;
            verify_pin(&mut guard, pin)?;
            drop(guard);
            true
        } else {
            false
        };
        Ok(Box::new(MockYubiDevice::new(card, actual, authorized)?))
    }

    fn open_admin(&self, locator: &YubiDeviceLocator) -> Result<Box<dyn YubiAdministrativeDevice>> {
        let card = self.card(&locator.card)?;
        let actual = locator_from_card(&card, locator.signing_slot, locator.pq_slot)?;
        if &actual != locator {
            return Err(Error::LocatorMismatch);
        }
        Ok(Box::new(MockYubiDevice::new(card, actual, false)?))
    }
}

fn verify_pin(card: &mut MockCard, pin: &Pin) -> Result<()> {
    if card.pin_remaining == 0 {
        return Err(Error::PinBlocked);
    }
    if card.pin != pin.expose() {
        card.pin_remaining -= 1;
        return Err(if card.pin_remaining == 0 {
            Error::PinBlocked
        } else {
            Error::PinRejected {
                remaining: card.pin_remaining,
            }
        });
    }
    card.pin_remaining = card.pin_attempts;
    Ok(())
}

fn configure_pin_retries(
    card: &mut MockCard,
    management_key: &ManagementKey,
    pin: &Pin,
    puk: &Pin,
    pin_attempts: u8,
    puk_attempts: u8,
) -> Result<()> {
    if &card.management_key != management_key.expose() {
        return Err(Error::Policy("management key was rejected"));
    }
    verify_pin(card, pin)?;
    // Match PIV SET PIN RETRIES: the command first resets both credentials.
    // This helper is called only before prepare generates any FOKS key.
    card.pin = "123456".to_owned();
    card.puk = "12345678".to_owned();
    card.pin_attempts = pin_attempts;
    card.pin_remaining = pin_attempts;
    card.puk_attempts = puk_attempts;
    card.puk_remaining = puk_attempts;
    #[cfg(test)]
    if card.fail_retry_update_unknown {
        return Err(Error::RetryUpdateUnknown);
    }
    #[cfg(test)]
    if card.fail_retry_pin_restore {
        return Err(Error::RetryPinRestore);
    }
    card.pin = pin.expose().to_owned();
    card.puk = puk.expose().to_owned();
    Ok(())
}

fn compressed(secret: &SecretKey) -> [u8; 33] {
    secret
        .public_key()
        .to_encoded_point(true)
        .as_bytes()
        .try_into()
        .expect("P-256 compressed points are 33 bytes")
}

fn self_secret(secret: &SecretKey) -> [u8; 32] {
    diffie_hellman(secret.to_nonzero_scalar(), secret.public_key().as_affine())
        .raw_secret_bytes()
        .as_slice()
        .try_into()
        .expect("P-256 ECDH output is 32 bytes")
}

fn locator_from_card(
    card: &Arc<Mutex<MockCard>>,
    signing_slot: SlotId,
    pq_slot: SlotId,
) -> Result<YubiDeviceLocator> {
    let card = card
        .lock()
        .map_err(|_| Error::Provider("mock card lock poisoned".into()))?;
    let signing = card.slots.get(&signing_slot).ok_or(Error::NotFound)?;
    let pq = card.slots.get(&pq_slot).ok_or(Error::NotFound)?;
    let signing_public_key = compressed(signing);
    let pq_public_key = compressed(pq);
    Ok(YubiDeviceLocator {
        card: card.id.clone(),
        signing_slot,
        pq_slot,
        signing_public_key,
        pq_public_key,
        pq_key_id: foks_crypto::yubi_pq_key_id(&pq_public_key)?,
    })
}

struct MockYubiDevice {
    card: Arc<Mutex<MockCard>>,
    locator: YubiDeviceLocator,
    id: EntityId,
    hepk: Hepk,
    authorized: bool,
}

impl MockYubiDevice {
    fn new(
        card: Arc<Mutex<MockCard>>,
        locator: YubiDeviceLocator,
        authorized: bool,
    ) -> Result<Self> {
        let material = {
            let card = card
                .lock()
                .map_err(|_| Error::Provider("mock card lock poisoned".into()))?;
            let pq = card.slots.get(&locator.pq_slot).ok_or(Error::NotFound)?;
            foks_crypto::derive_yubi_public_material(
                locator.signing_public_key,
                locator.pq_public_key,
                self_secret(pq),
            )?
        };
        if material.pq_key_id != locator.pq_key_id {
            return Err(Error::LocatorMismatch);
        }
        Ok(Self {
            card,
            locator,
            id: material.device.id,
            hepk: material.device.hepk,
            authorized,
        })
    }

    fn require_authorized(&self) -> std::result::Result<(), foks_crypto::Error> {
        self.authorized
            .then_some(())
            .ok_or(foks_crypto::Error::YubiSigning)
    }
}

impl HybridSecretDecapsulator for MockYubiDevice {
    fn entity_id(&self) -> &EntityId {
        &self.id
    }

    fn hepk(&self) -> &Hepk {
        &self.hepk
    }

    fn derive_dh_shared(
        &self,
        peer: &DhPublicKey,
    ) -> std::result::Result<Zeroizing<[u8; 32]>, foks_crypto::Error> {
        self.require_authorized()?;
        let DhPublicKey::P256(peer) = peer else {
            return Err(foks_crypto::Error::HybridBox);
        };
        let peer = PublicKey::from_sec1_bytes(peer).map_err(|_| foks_crypto::Error::HybridBox)?;
        let card = self
            .card
            .lock()
            .map_err(|_| foks_crypto::Error::YubiSigning)?;
        let secret = card
            .slots
            .get(&self.locator.signing_slot)
            .ok_or(foks_crypto::Error::YubiSigning)?;
        let shared = diffie_hellman(secret.to_nonzero_scalar(), peer.as_affine());
        Ok(Zeroizing::new(
            shared
                .raw_secret_bytes()
                .as_slice()
                .try_into()
                .map_err(|_| foks_crypto::Error::HybridBox)?,
        ))
    }

    fn decapsulate_mlkem768(
        &self,
        ciphertext: &[u8],
    ) -> std::result::Result<Zeroizing<[u8; 32]>, foks_crypto::Error> {
        self.require_authorized()?;
        let card = self
            .card
            .lock()
            .map_err(|_| foks_crypto::Error::YubiSigning)?;
        let pq = card
            .slots
            .get(&self.locator.pq_slot)
            .ok_or(foks_crypto::Error::YubiSigning)?;
        foks_crypto::yubi_mlkem_decapsulate(self_secret(pq), ciphertext)
    }
}

impl YubiDevice for MockYubiDevice {
    fn pq_key_id(&self) -> [u8; 32] {
        self.locator.pq_key_id
    }

    fn sign_sha512_256(
        &self,
        digest: &[u8; 32],
    ) -> std::result::Result<Vec<u8>, foks_crypto::Error> {
        self.require_authorized()?;
        let card = self
            .card
            .lock()
            .map_err(|_| foks_crypto::Error::YubiSigning)?;
        let secret = card
            .slots
            .get(&self.locator.signing_slot)
            .ok_or(foks_crypto::Error::YubiSigning)?;
        let signing = SigningKey::from(secret.clone());
        let signature: P256Signature = signing
            .sign_prehash(digest)
            .map_err(|_| foks_crypto::Error::YubiSigning)?;
        Ok(signature.to_der().as_bytes().to_vec())
    }
}

impl YubiAdministrativeDevice for MockYubiDevice {
    fn locator(&self) -> &YubiDeviceLocator {
        &self.locator
    }

    fn pin_retries(&self) -> Result<PinRetries> {
        let card = self
            .card
            .lock()
            .map_err(|_| Error::Provider("mock card lock poisoned".into()))?;
        Ok(PinRetries {
            remaining: card.pin_remaining,
            blocked: card.pin_remaining == 0,
        })
    }

    fn change_pin(&self, old: &Pin, new: &Pin) -> Result<()> {
        let mut card = self
            .card
            .lock()
            .map_err(|_| Error::Provider("mock card lock poisoned".into()))?;
        verify_pin(&mut card, old)?;
        card.pin = new.expose().to_owned();
        Ok(())
    }

    fn change_puk(&self, old: &Pin, new: &Pin) -> Result<()> {
        let mut card = self
            .card
            .lock()
            .map_err(|_| Error::Provider("mock card lock poisoned".into()))?;
        if card.puk_remaining == 0 {
            return Err(Error::PinBlocked);
        }
        if card.puk != old.expose() {
            card.puk_remaining = card.puk_remaining.saturating_sub(1);
            return Err(if card.puk_remaining == 0 {
                Error::PinBlocked
            } else {
                Error::PinRejected {
                    remaining: card.puk_remaining,
                }
            });
        }
        card.puk = new.expose().to_owned();
        card.puk_remaining = card.puk_attempts;
        Ok(())
    }

    fn unblock_pin(&self, puk: &Pin, new_pin: &Pin) -> Result<()> {
        let mut card = self
            .card
            .lock()
            .map_err(|_| Error::Provider("mock card lock poisoned".into()))?;
        if card.puk_remaining == 0 {
            return Err(Error::PinBlocked);
        }
        if card.puk != puk.expose() {
            card.puk_remaining = card.puk_remaining.saturating_sub(1);
            return Err(if card.puk_remaining == 0 {
                Error::PinBlocked
            } else {
                Error::PinRejected {
                    remaining: card.puk_remaining,
                }
            });
        }
        card.pin = new_pin.expose().to_owned();
        card.pin_remaining = card.pin_attempts;
        card.puk_remaining = card.puk_attempts;
        Ok(())
    }

    fn replace_management_key(
        &self,
        old: &ManagementKey,
        new: &ManagementKey,
        _store_with_pin: bool,
    ) -> Result<()> {
        let mut card = self
            .card
            .lock()
            .map_err(|_| Error::Provider("mock card lock poisoned".into()))?;
        if &card.management_key != old.expose() {
            return Err(Error::Policy("management key was rejected"));
        }
        card.management_key.copy_from_slice(new.expose());
        Ok(())
    }
}

impl ManagedYubiDevice for MockYubiDevice {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifecycle_binds_slots_and_never_retries_a_bad_pin() {
        let pin = Pin::new("123456").unwrap();
        let provider = MockYubiProvider::with_card("mock", 7, &pin).unwrap();
        let prepared = provider
            .prepare(
                &provider.cards().unwrap()[0],
                SlotId::new(0x82).unwrap(),
                SlotId::new(0x83).unwrap(),
                &pin,
                None,
                PivPolicy::Once,
                PivPolicy::Never,
            )
            .unwrap();
        assert_eq!(prepared.entity_id, *prepared.device.entity_id());
        let wrong = Pin::new("654321").unwrap();
        assert!(matches!(
            provider.open(&prepared.locator, Some(&wrong)),
            Err(Error::PinRejected { remaining: 2 })
        ));
        assert_eq!(
            provider
                .open(&prepared.locator, Some(&pin))
                .unwrap()
                .pin_retries()
                .unwrap()
                .remaining,
            3
        );
    }

    #[test]
    fn administrative_handle_unblocks_a_pin_without_crypto_authorization() {
        let pin = Pin::new("123456").unwrap();
        let provider = MockYubiProvider::with_card("mock", 8, &pin).unwrap();
        let prepared = provider
            .prepare(
                &provider.cards().unwrap()[0],
                SlotId::new(0x82).unwrap(),
                SlotId::new(0x83).unwrap(),
                &pin,
                None,
                PivPolicy::Once,
                PivPolicy::Never,
            )
            .unwrap();
        for expected in [2, 1] {
            assert!(matches!(
                provider.open(&prepared.locator, Some(&Pin::new("654321").unwrap())),
                Err(Error::PinRejected { remaining }) if remaining == expected
            ));
        }
        assert!(matches!(
            provider.open(&prepared.locator, Some(&Pin::new("654321").unwrap())),
            Err(Error::PinBlocked)
        ));
        let admin = provider.open_admin(&prepared.locator).unwrap();
        assert!(admin.pin_retries().unwrap().blocked);
        admin
            .unblock_pin(&Pin::new("12345678").unwrap(), &Pin::new("234567").unwrap())
            .unwrap();
        assert_eq!(admin.pin_retries().unwrap().remaining, 3);
        provider
            .open(&prepared.locator, Some(&Pin::new("234567").unwrap()))
            .unwrap();
    }

    #[test]
    fn retry_configuration_precedes_key_generation_and_restores_credentials() {
        let pin = Pin::new("pin-42").unwrap();
        let provider = MockYubiProvider::with_card("mock", 9, &pin).unwrap();
        let retry = PinRetryConfiguration::new(Pin::new("puk-42").unwrap(), 5, 4).unwrap();
        assert!(!format!("{retry:?}").contains("puk-42"));
        let prepared = provider
            .prepare(
                &provider.cards().unwrap()[0],
                SlotId::new(0x82).unwrap(),
                SlotId::new(0x83).unwrap(),
                &pin,
                Some(&retry),
                PivPolicy::Once,
                PivPolicy::Never,
            )
            .unwrap();
        let admin = provider.open_admin(&prepared.locator).unwrap();

        assert_eq!(admin.pin_retries().unwrap().remaining, 5);
        provider
            .open(&prepared.locator, Some(&pin))
            .expect("the nondefault PIN must survive retry configuration");
        admin
            .unblock_pin(&Pin::new("puk-42").unwrap(), &Pin::new("new-42").unwrap())
            .expect("the nondefault PUK must survive retry configuration");
    }

    #[test]
    fn retry_configuration_is_rejected_after_foks_key_generation() {
        let pin = Pin::new("123456").unwrap();
        let provider = MockYubiProvider::with_card("mock", 10, &pin).unwrap();
        let card = provider.cards().unwrap().remove(0);
        let prepared = provider
            .prepare(
                &card,
                SlotId::new(0x82).unwrap(),
                SlotId::new(0x83).unwrap(),
                &pin,
                None,
                PivPolicy::Once,
                PivPolicy::Never,
            )
            .unwrap();
        let retry = PinRetryConfiguration::new(Pin::new("12345678").unwrap(), 5, 4).unwrap();

        assert!(matches!(
            provider.prepare(
                &card,
                SlotId::new(0x84).unwrap(),
                SlotId::new(0x85).unwrap(),
                &pin,
                Some(&retry),
                PivPolicy::Once,
                PivPolicy::Never,
            ),
            Err(Error::Policy(
                "PIV retry configuration requires every key slot to be empty"
            ))
        ));
        assert_eq!(
            provider
                .open_admin(&prepared.locator)
                .unwrap()
                .pin_retries()
                .unwrap()
                .remaining,
            3
        );
    }

    #[test]
    fn unknown_retry_update_does_not_generate_foks_keys() {
        let pin = Pin::new("pin-42").unwrap();
        let provider = MockYubiProvider::with_card("mock", 11, &pin).unwrap();
        let card = provider.cards().unwrap().remove(0);
        let state = provider.card(&card).unwrap();
        state.lock().unwrap().fail_retry_update_unknown = true;
        let retry = PinRetryConfiguration::new(Pin::new("puk-42").unwrap(), 5, 4).unwrap();

        assert!(matches!(
            provider.prepare(
                &card,
                SlotId::new(0x82).unwrap(),
                SlotId::new(0x83).unwrap(),
                &pin,
                Some(&retry),
                PivPolicy::Once,
                PivPolicy::Never,
            ),
            Err(Error::RetryUpdateUnknown)
        ));
        assert!(state.lock().unwrap().slots.is_empty());
    }

    #[test]
    fn failed_retry_restore_does_not_generate_foks_keys() {
        let pin = Pin::new("pin-42").unwrap();
        let provider = MockYubiProvider::with_card("mock", 12, &pin).unwrap();
        let card = provider.cards().unwrap().remove(0);
        let state = provider.card(&card).unwrap();
        state.lock().unwrap().fail_retry_pin_restore = true;
        let retry = PinRetryConfiguration::new(Pin::new("puk-42").unwrap(), 5, 4).unwrap();

        assert!(matches!(
            provider.prepare(
                &card,
                SlotId::new(0x82).unwrap(),
                SlotId::new(0x83).unwrap(),
                &pin,
                Some(&retry),
                PivPolicy::Once,
                PivPolicy::Never,
            ),
            Err(Error::RetryPinRestore)
        ));
        let state = state.lock().unwrap();
        assert!(state.slots.is_empty());
        assert_eq!(state.pin, "123456");
        assert_eq!(state.puk, "12345678");
        assert_eq!((state.pin_attempts, state.puk_attempts), (5, 4));
    }

    #[test]
    fn retired_slot_deserialization_preserves_the_range_invariant() {
        assert!(serde_json::from_str::<SlotId>("129").is_err());
        assert_eq!(
            serde_json::from_str::<SlotId>("130").unwrap(),
            SlotId::new(0x82).unwrap()
        );
        assert!(serde_json::from_str::<SlotId>("150").is_err());
    }
}
