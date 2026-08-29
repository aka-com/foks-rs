//! Native macOS/Linux PC/SC adapter.
//!
//! No `YubiKey` handle is retained across calls. Each operation reopens the
//! exact serial number, verifies its PIN when required, and holds a provider
//! lock so concurrent operations cannot interleave APDUs on the same process.

use std::collections::BTreeSet;
use std::sync::{Mutex, MutexGuard};

use foks_crypto::{HybridSecretDecapsulator, YubiDevice};
use foks_proto::{DhPublicKey, EntityId, Hepk};
use p256::ecdsa::{signature::hazmat::PrehashVerifier as _, Signature, VerifyingKey};
use p256::elliptic_curve::sec1::ToEncodedPoint as _;
use p256::PublicKey;
use sha2::{Digest as _, Sha512_256};
use yubikey::piv::{self, AlgorithmId};
use yubikey::{MgmKey, PinPolicy, Serial, TouchPolicy, YubiKey};
use zeroize::Zeroizing;

use crate::{
    CardId, Error, ManagedYubiDevice, ManagementKey, Pin, PinRetries, PinRetryConfiguration,
    PivPolicy, PreparedYubiDevice, Result, SlotId, YubiAdministrativeDevice, YubiDeviceLocator,
    YubiProvider,
};

static HARDWARE_OPERATION_LOCK: Mutex<()> = Mutex::new(());

#[derive(Clone, Copy, Default)]
pub struct HardwareYubiProvider;

impl HardwareYubiProvider {
    pub fn new() -> Self {
        Self
    }

    fn lock(&self) -> Result<MutexGuard<'static, ()>> {
        HARDWARE_OPERATION_LOCK
            .lock()
            .map_err(|_| Error::Provider("hardware operation lock poisoned".into()))
    }

    fn open_card(card: &CardId) -> Result<YubiKey> {
        let yubikey = YubiKey::open_by_serial(Serial::from(card.serial)).map_err(map_error)?;
        if yubikey.name() != card.name || u32::from(yubikey.serial()) != card.serial {
            return Err(Error::LocatorMismatch);
        }
        Ok(yubikey)
    }
}

impl YubiProvider for HardwareYubiProvider {
    fn cards(&self) -> Result<Vec<CardId>> {
        let _guard = self.lock()?;
        let mut context = yubikey::reader::Context::open().map_err(map_error)?;
        let mut seen = BTreeSet::new();
        let mut cards = Vec::new();
        for reader in context.iter().map_err(map_error)? {
            let Ok(yubikey) = reader.open() else {
                continue;
            };
            let serial = u32::from(yubikey.serial());
            if seen.insert(serial) {
                cards.push(CardId {
                    name: yubikey.name().to_owned(),
                    serial,
                });
            }
        }
        cards.sort();
        Ok(cards)
    }

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
        if signing_slot == pq_slot {
            return Err(Error::Policy("signing and PQ slots must be distinct"));
        }
        let _guard = self.lock()?;
        let mut yubikey = Self::open_card(card)?;
        verify_pin(&mut yubikey, pin)?;
        // Provisioning intentionally requires a factory/default management key.
        // Existing managed cards must be reset or enrolled through a future
        // import flow rather than having slots overwritten implicitly.
        yubikey.authenticate(MgmKey::default()).map_err(map_error)?;

        let signing_slot_native = native_slot(signing_slot)?;
        let pq_slot_native = native_slot(pq_slot)?;
        if let Some(retry) = retry_configuration {
            ensure_all_piv_slots_empty(&mut yubikey)?;
            configure_pin_retries(
                &mut yubikey,
                &ManagementKey::default_piv(),
                pin,
                &retry.puk,
                retry.pin_attempts,
                retry.puk_attempts,
            )?;
        } else {
            ensure_slot_empty(&mut yubikey, signing_slot_native)?;
            ensure_slot_empty(&mut yubikey, pq_slot_native)?;
        }
        let signing_spki = piv::generate(
            &mut yubikey,
            signing_slot_native,
            AlgorithmId::EccP256,
            native_pin_policy(pin_policy),
            native_touch_policy(touch_policy),
        )
        .map_err(map_error)?;
        let pq_spki = piv::generate(
            &mut yubikey,
            pq_slot_native,
            AlgorithmId::EccP256,
            native_pin_policy(pin_policy),
            native_touch_policy(touch_policy),
        )
        .map_err(map_error)?;
        let signing_public_key = compressed_public(signing_spki.subject_public_key.raw_bytes())?;
        let pq_public_key = compressed_public(pq_spki.subject_public_key.raw_bytes())?;
        let locator = YubiDeviceLocator {
            card: card.clone(),
            signing_slot,
            pq_slot,
            signing_public_key,
            pq_public_key,
            pq_key_id: foks_crypto::yubi_pq_key_id(&pq_public_key)?,
        };
        drop(yubikey);
        drop(_guard);
        let device = HardwareYubiDevice::open(
            locator.clone(),
            Some(Zeroizing::new(pin.expose().as_bytes().to_vec())),
        )?;
        Ok(PreparedYubiDevice {
            entity_id: device.id.clone(),
            hepk: device.hepk.clone(),
            locator,
            device: Box::new(device),
        })
    }

    fn open(
        &self,
        locator: &YubiDeviceLocator,
        pin: Option<&Pin>,
    ) -> Result<Box<dyn ManagedYubiDevice>> {
        Ok(Box::new(HardwareYubiDevice::open(
            locator.clone(),
            pin.map(|pin| Zeroizing::new(pin.expose().as_bytes().to_vec())),
        )?))
    }

    fn open_admin(&self, locator: &YubiDeviceLocator) -> Result<Box<dyn YubiAdministrativeDevice>> {
        if locator.signing_slot == locator.pq_slot
            || foks_crypto::yubi_pq_key_id(&locator.pq_public_key)? != locator.pq_key_id
        {
            return Err(Error::LocatorMismatch);
        }
        let _guard = self.lock()?;
        let mut card = Self::open_card(&locator.card)?;
        verify_locator_slots(&mut card, locator)?;
        Ok(Box::new(HardwareYubiAdminDevice {
            locator: locator.clone(),
        }))
    }
}

struct HardwareYubiAdminDevice {
    locator: YubiDeviceLocator,
}

impl HardwareYubiAdminDevice {
    fn with_card<T>(&self, operation: impl FnOnce(&mut YubiKey) -> Result<T>) -> Result<T> {
        let _guard = HARDWARE_OPERATION_LOCK
            .lock()
            .map_err(|_| Error::Provider("hardware operation lock poisoned".into()))?;
        let mut yubikey = HardwareYubiProvider::open_card(&self.locator.card)?;
        verify_locator_slots(&mut yubikey, &self.locator)?;
        operation(&mut yubikey)
    }
}

impl YubiAdministrativeDevice for HardwareYubiAdminDevice {
    fn locator(&self) -> &YubiDeviceLocator {
        &self.locator
    }

    fn pin_retries(&self) -> Result<PinRetries> {
        self.with_card(|yubikey| {
            let remaining = yubikey.get_pin_retries().map_err(map_error)?;
            Ok(PinRetries {
                remaining,
                blocked: remaining == 0,
            })
        })
    }

    fn change_pin(&self, old: &Pin, new: &Pin) -> Result<()> {
        self.with_card(|yubikey| {
            yubikey
                .change_pin(old.expose().as_bytes(), new.expose().as_bytes())
                .map_err(map_error)
        })
    }

    fn change_puk(&self, old: &Pin, new: &Pin) -> Result<()> {
        self.with_card(|yubikey| {
            yubikey
                .change_puk(old.expose().as_bytes(), new.expose().as_bytes())
                .map_err(map_error)
        })
    }

    fn unblock_pin(&self, puk: &Pin, new_pin: &Pin) -> Result<()> {
        self.with_card(|yubikey| {
            yubikey
                .unblock_pin(puk.expose().as_bytes(), new_pin.expose().as_bytes())
                .map_err(map_error)
        })
    }

    fn replace_management_key(
        &self,
        old: &ManagementKey,
        new: &ManagementKey,
        store_with_pin: bool,
    ) -> Result<()> {
        self.with_card(|yubikey| {
            let old = MgmKey::from_bytes(old.expose()).map_err(map_error)?;
            yubikey.authenticate(old).map_err(map_error)?;
            let new = MgmKey::from_bytes(new.expose()).map_err(map_error)?;
            if store_with_pin {
                new.set_protected(yubikey).map_err(map_error)
            } else {
                new.set_manual(yubikey, false).map_err(map_error)
            }
        })
    }
}

struct HardwareYubiDevice {
    locator: YubiDeviceLocator,
    id: EntityId,
    hepk: Hepk,
    pin: Option<Zeroizing<Vec<u8>>>,
}

impl HardwareYubiDevice {
    fn open(locator: YubiDeviceLocator, pin: Option<Zeroizing<Vec<u8>>>) -> Result<Self> {
        if locator.signing_slot == locator.pq_slot
            || foks_crypto::yubi_pq_key_id(&locator.pq_public_key)? != locator.pq_key_id
        {
            return Err(Error::LocatorMismatch);
        }
        let guard = HARDWARE_OPERATION_LOCK
            .lock()
            .map_err(|_| Error::Provider("hardware operation lock poisoned".into()))?;
        let mut yubikey = HardwareYubiProvider::open_card(&locator.card)?;
        if let Some(pin) = pin.as_deref() {
            verify_pin_bytes(&mut yubikey, pin)?;
        }
        prove_slot(
            &mut yubikey,
            locator.signing_slot,
            &locator.signing_public_key,
        )?;
        prove_slot(&mut yubikey, locator.pq_slot, &locator.pq_public_key)?;
        let pq_self_secret = ecdh(&mut yubikey, locator.pq_slot, &locator.pq_public_key)?;
        let material = foks_crypto::derive_yubi_public_material(
            locator.signing_public_key,
            locator.pq_public_key,
            pq_self_secret,
        )?;
        drop(yubikey);
        drop(guard);
        Ok(Self {
            locator,
            id: material.device.id,
            hepk: material.device.hepk,
            pin,
        })
    }

    fn with_card<T>(&self, operation: impl FnOnce(&mut YubiKey) -> Result<T>) -> Result<T> {
        let _guard = HARDWARE_OPERATION_LOCK
            .lock()
            .map_err(|_| Error::Provider("hardware operation lock poisoned".into()))?;
        let mut yubikey = HardwareYubiProvider::open_card(&self.locator.card)?;
        verify_locator_slots(&mut yubikey, &self.locator)?;
        if let Some(pin) = self.pin.as_deref() {
            verify_pin_bytes(&mut yubikey, pin)?;
        }
        operation(&mut yubikey)
    }
}

impl HybridSecretDecapsulator for HardwareYubiDevice {
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
        let DhPublicKey::P256(peer) = peer else {
            return Err(foks_crypto::Error::HybridBox);
        };
        self.with_card(|yubikey| ecdh(yubikey, self.locator.signing_slot, peer))
            .map(Zeroizing::new)
            .map_err(|_| foks_crypto::Error::HybridBox)
    }

    fn decapsulate_mlkem768(
        &self,
        ciphertext: &[u8],
    ) -> std::result::Result<Zeroizing<[u8; 32]>, foks_crypto::Error> {
        let self_secret = self
            .with_card(|yubikey| ecdh(yubikey, self.locator.pq_slot, &self.locator.pq_public_key))
            .map_err(|_| foks_crypto::Error::HybridBox)?;
        foks_crypto::yubi_mlkem_decapsulate(self_secret, ciphertext)
    }
}

impl YubiDevice for HardwareYubiDevice {
    fn pq_key_id(&self) -> [u8; 32] {
        self.locator.pq_key_id
    }

    fn sign_sha512_256(
        &self,
        digest: &[u8; 32],
    ) -> std::result::Result<Vec<u8>, foks_crypto::Error> {
        self.with_card(|yubikey| {
            piv::sign_data(
                yubikey,
                digest,
                AlgorithmId::EccP256,
                native_slot(self.locator.signing_slot)?,
            )
            .map(|signature| signature.to_vec())
            .map_err(map_error)
        })
        .map_err(|_| foks_crypto::Error::YubiSigning)
    }
}

impl YubiAdministrativeDevice for HardwareYubiDevice {
    fn locator(&self) -> &YubiDeviceLocator {
        &self.locator
    }

    fn pin_retries(&self) -> Result<PinRetries> {
        self.with_card(|yubikey| {
            let remaining = yubikey.get_pin_retries().map_err(map_error)?;
            Ok(PinRetries {
                remaining,
                blocked: remaining == 0,
            })
        })
    }

    fn change_pin(&self, old: &Pin, new: &Pin) -> Result<()> {
        self.with_card(|yubikey| {
            yubikey
                .change_pin(old.expose().as_bytes(), new.expose().as_bytes())
                .map_err(map_error)
        })
    }

    fn change_puk(&self, old: &Pin, new: &Pin) -> Result<()> {
        self.with_card(|yubikey| {
            yubikey
                .change_puk(old.expose().as_bytes(), new.expose().as_bytes())
                .map_err(map_error)
        })
    }

    fn unblock_pin(&self, puk: &Pin, new_pin: &Pin) -> Result<()> {
        self.with_card(|yubikey| {
            yubikey
                .unblock_pin(puk.expose().as_bytes(), new_pin.expose().as_bytes())
                .map_err(map_error)
        })
    }

    fn replace_management_key(
        &self,
        old: &ManagementKey,
        new: &ManagementKey,
        store_with_pin: bool,
    ) -> Result<()> {
        self.with_card(|yubikey| {
            let old = MgmKey::from_bytes(old.expose()).map_err(map_error)?;
            yubikey.authenticate(old).map_err(map_error)?;
            let new = MgmKey::from_bytes(new.expose()).map_err(map_error)?;
            if store_with_pin {
                new.set_protected(yubikey).map_err(map_error)
            } else {
                new.set_manual(yubikey, false).map_err(map_error)
            }
        })
    }
}

impl ManagedYubiDevice for HardwareYubiDevice {}

fn verify_pin(yubikey: &mut YubiKey, pin: &Pin) -> Result<()> {
    verify_pin_bytes(yubikey, pin.expose().as_bytes())
}

fn verify_pin_bytes(yubikey: &mut YubiKey, pin: &[u8]) -> Result<()> {
    yubikey.verify_pin(pin).map_err(map_error)
}

fn configure_pin_retries(
    yubikey: &mut YubiKey,
    management_key: &ManagementKey,
    pin: &Pin,
    puk: &Pin,
    pin_attempts: u8,
    puk_attempts: u8,
) -> Result<()> {
    let management_key = MgmKey::from_bytes(management_key.expose()).map_err(map_error)?;
    yubikey.authenticate(management_key).map_err(map_error)?;
    verify_pin(yubikey, pin)?;
    yubikey
        .set_pin_retries(pin_attempts, puk_attempts)
        .map_err(|_| Error::RetryUpdateUnknown)?;
    yubikey
        .change_pin(b"123456", pin.expose().as_bytes())
        .map_err(|_| Error::RetryPinRestore)?;
    yubikey
        .change_puk(b"12345678", puk.expose().as_bytes())
        .map_err(|_| Error::RetryPukRestore)
}

fn native_slot(slot: SlotId) -> Result<piv::SlotId> {
    piv::SlotId::try_from(slot.get()).map_err(map_error)
}

fn native_pin_policy(policy: PivPolicy) -> PinPolicy {
    match policy {
        PivPolicy::Never => PinPolicy::Never,
        PivPolicy::Once => PinPolicy::Once,
        PivPolicy::Always => PinPolicy::Always,
    }
}

fn native_touch_policy(policy: PivPolicy) -> TouchPolicy {
    match policy {
        PivPolicy::Never => TouchPolicy::Never,
        PivPolicy::Once => TouchPolicy::Cached,
        PivPolicy::Always => TouchPolicy::Always,
    }
}

fn ensure_slot_empty(yubikey: &mut YubiKey, slot: piv::SlotId) -> Result<()> {
    match piv::metadata(yubikey, slot) {
        Ok(metadata) if metadata.public.is_some() => {
            Err(Error::Policy("selected PIV slot is not empty"))
        }
        Ok(_) | Err(yubikey::Error::NotFound) => Ok(()),
        // Firmware before 5.2.3 cannot report slot metadata. Refuse to
        // overwrite because absence cannot be established safely.
        Err(yubikey::Error::NotSupported) => Err(Error::Policy(
            "YubiKey firmware cannot prove that the selected slot is empty",
        )),
        Err(error) => Err(map_error(error)),
    }
}

fn ensure_all_piv_slots_empty(yubikey: &mut YubiKey) -> Result<()> {
    for slot in [
        piv::SlotId::Authentication,
        piv::SlotId::Signature,
        piv::SlotId::KeyManagement,
        piv::SlotId::CardAuthentication,
    ] {
        ensure_slot_empty(yubikey, slot)?;
    }
    for value in SlotId::MIN..=SlotId::MAX {
        ensure_slot_empty(yubikey, native_slot(SlotId::new(value)?)?)?;
    }
    Ok(())
}

fn verify_slot_public(yubikey: &mut YubiKey, slot: SlotId, expected: &[u8; 33]) -> Result<()> {
    let metadata = piv::metadata(yubikey, native_slot(slot)?).map_err(map_error)?;
    let public = metadata.public.ok_or(Error::LocatorMismatch)?;
    if compressed_public(public.subject_public_key.raw_bytes())? != *expected {
        return Err(Error::LocatorMismatch);
    }
    Ok(())
}

fn verify_locator_slots(yubikey: &mut YubiKey, locator: &YubiDeviceLocator) -> Result<()> {
    verify_slot_public(yubikey, locator.signing_slot, &locator.signing_public_key)?;
    verify_slot_public(yubikey, locator.pq_slot, &locator.pq_public_key)
}

fn compressed_public(bytes: &[u8]) -> Result<[u8; 33]> {
    let public = PublicKey::from_sec1_bytes(bytes)
        .map_err(|_| Error::Provider("YubiKey returned an invalid P-256 public key".into()))?;
    public
        .to_encoded_point(true)
        .as_bytes()
        .try_into()
        .map_err(|_| Error::Provider("YubiKey returned an invalid P-256 public key".into()))
}

fn prove_slot(yubikey: &mut YubiKey, slot: SlotId, public: &[u8; 33]) -> Result<()> {
    let digest: [u8; 32] = Sha512_256::digest(b"foks-yubi-slot-binding-v1").into();
    let signature = piv::sign_data(yubikey, &digest, AlgorithmId::EccP256, native_slot(slot)?)
        .map_err(map_error)?;
    let key = VerifyingKey::from_sec1_bytes(public).map_err(|_| Error::LocatorMismatch)?;
    let signature = Signature::from_der(&signature).map_err(|_| Error::LocatorMismatch)?;
    key.verify_prehash(&digest, &signature)
        .map_err(|_| Error::LocatorMismatch)
}

fn ecdh(yubikey: &mut YubiKey, slot: SlotId, peer: &[u8; 33]) -> Result<[u8; 32]> {
    let peer = PublicKey::from_sec1_bytes(peer)
        .map_err(|_| Error::Provider("invalid P-256 ECDH peer".into()))?;
    let uncompressed = peer.to_encoded_point(false);
    piv::decrypt_data(
        yubikey,
        uncompressed.as_bytes(),
        AlgorithmId::EccP256,
        native_slot(slot)?,
    )
    .map_err(map_error)?
    .as_slice()
    .try_into()
    .map_err(|_| Error::Provider("YubiKey returned an invalid ECDH secret".into()))
}

fn map_error(error: yubikey::Error) -> Error {
    match error {
        yubikey::Error::NotFound => Error::NotFound,
        yubikey::Error::WrongPin { tries } => Error::PinRejected { remaining: tries },
        yubikey::Error::PinLocked => Error::PinBlocked,
        error => Error::Provider(error.to_string()),
    }
}
