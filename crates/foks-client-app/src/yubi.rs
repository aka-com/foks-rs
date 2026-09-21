//! Product-level Yubi enrollment, protected records, and PIV lifecycle.

use foks_client::{
    decrypt_yubi_management_key, encrypt_yubi_management_key, NewYubiDeviceSecrets,
    YubiAccountRequest, YubiAccountSecrets, YubiCredential, YubiDeviceProvisionRequest,
};
use foks_crypto::{derive_subkey_id, YubiDevice};
use foks_proto::{EntityId, InviteCode, Role, SecretSeed, YubiCardId, YubiSlotAndPqKeyId};
use foks_yubi::{
    CardId, ManagementKey, Pin, PinRetries, PinRetryConfiguration, PivPolicy, PreparedYubiDevice,
    SlotId, YubiDeviceLocator, YubiProvider,
};
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize as _, Zeroizing};

use super::account::validate_certificates;
use super::*;

const YUBI_RECORD_VERSION: u32 = 1;
const YUBI_REFRESH_JOB_TYPE_ID: u64 = 0xd258_a5e4_594d_4b31;

#[derive(Deserialize, Serialize)]
pub(super) struct YubiRefreshScope {
    pub yubi_alias: String,
    pub software_alias: String,
}

pub(super) fn yubi_alias_from_key(key: &str) -> Option<&str> {
    key.strip_prefix("yubi-account.")
        .or_else(|| key.strip_prefix("pending-yubi."))
}

pub(super) fn yubi_account_key(alias: &str) -> String {
    format!("yubi-account.{alias}")
}

pub(super) fn pending_yubi_key(alias: &str) -> String {
    format!("pending-yubi.{alias}")
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
enum PendingYubiPurpose {
    Signup {
        device_name: String,
        email: String,
        invite: String,
        passphrase: Option<Vec<u8>>,
    },
    Provision {
        source_alias: String,
        device_name: String,
        serial: u64,
    },
}

impl Drop for PendingYubiPurpose {
    fn drop(&mut self) {
        if let Self::Signup {
            invite, passphrase, ..
        } = self
        {
            invite.zeroize();
            passphrase.zeroize();
        }
    }
}

#[derive(Deserialize, Serialize)]
struct PendingYubiRetryConfiguration {
    puk: Vec<u8>,
    pin_attempts: u8,
    puk_attempts: u8,
}

impl Drop for PendingYubiRetryConfiguration {
    fn drop(&mut self) {
        self.puk.zeroize();
    }
}

impl PendingYubiRetryConfiguration {
    fn new(configuration: PinRetryConfiguration) -> Self {
        Self {
            puk: configuration.puk().expose().as_bytes().to_vec(),
            pin_attempts: configuration.pin_attempts(),
            puk_attempts: configuration.puk_attempts(),
        }
    }

    fn puk(&self) -> Result<Pin> {
        let value = std::str::from_utf8(&self.puk)
            .map_err(|_| Error::InvalidAccount("pending Yubi PUK is invalid"))?;
        Ok(Pin::new(value.to_owned())?)
    }
}

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
enum PendingYubiPreparationCheckpoint {
    Planned,
    RetryResetPending,
    PinRestorePending,
    PukRestorePending,
    CredentialsReady,
    SigningKeyPending,
    SigningKeyReady {
        #[serde(with = "array33")]
        signing_public_key: [u8; 33],
    },
    PqKeyPending {
        #[serde(with = "array33")]
        signing_public_key: [u8; 33],
    },
    KeysReady {
        #[serde(with = "array33")]
        signing_public_key: [u8; 33],
        #[serde(with = "array33")]
        pq_public_key: [u8; 33],
    },
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

#[derive(Deserialize, Serialize)]
struct PendingYubiPreparation {
    card: CardId,
    signing_slot: SlotId,
    pq_slot: SlotId,
    retry_configuration: Option<PendingYubiRetryConfiguration>,
    pin_policy: PivPolicy,
    touch_policy: PivPolicy,
    checkpoint: PendingYubiPreparationCheckpoint,
}

impl PendingYubiPreparation {
    fn new(
        card: CardId,
        signing_slot: SlotId,
        pq_slot: SlotId,
        retry_configuration: Option<PinRetryConfiguration>,
    ) -> Self {
        Self {
            card,
            signing_slot,
            pq_slot,
            retry_configuration: retry_configuration.map(PendingYubiRetryConfiguration::new),
            pin_policy: PivPolicy::Once,
            touch_policy: PivPolicy::Never,
            checkpoint: PendingYubiPreparationCheckpoint::Planned,
        }
    }
}

#[derive(Deserialize, Serialize)]
struct StoredYubiAccount {
    version: u32,
    alias: String,
    username: String,
    uid: Vec<u8>,
    locator: YubiDeviceLocator,
    subkey_id: Vec<u8>,
    subkey_seed: [u8; 32],
    certificate_chain: Vec<Vec<u8>>,
    management_key: Option<[u8; 24]>,
    pending_management_key: Option<[u8; 24]>,
    management_enrolled: bool,
    management_generation: Option<u64>,
    management_refresh_source: Option<String>,
}

impl Drop for StoredYubiAccount {
    fn drop(&mut self) {
        self.subkey_seed.zeroize();
        self.management_key.zeroize();
        self.pending_management_key.zeroize();
    }
}

impl StoredYubiAccount {
    fn management_key_for_publication(&self) -> Result<[u8; 24]> {
        self.pending_management_key
            .or(self.management_key)
            .ok_or(Error::InvalidAccount("Yubi management key is unavailable"))
    }

    fn require_completed_management_rotation(&self) -> Result<()> {
        if self.pending_management_key.is_some() || !self.management_enrolled {
            return Err(Error::InvalidAccount(
                "Yubi management-key rotation is incomplete; resume it first",
            ));
        }
        Ok(())
    }

    fn complete_management_publication(&mut self, generation: u64) {
        if let Some(next) = self.pending_management_key.take() {
            self.management_key = Some(next);
        }
        self.management_enrolled = true;
        self.management_generation = Some(generation);
    }
}

#[derive(Deserialize, Serialize)]
struct PendingYubiAccount {
    version: u32,
    alias: String,
    username: String,
    locator: Option<YubiDeviceLocator>,
    preparation: Option<PendingYubiPreparation>,
    subkey_seed: [u8; 32],
    puk_seed: Option<[u8; 32]>,
    self_token: [u8; 17],
    purpose: PendingYubiPurpose,
}

impl Drop for PendingYubiAccount {
    fn drop(&mut self) {
        self.subkey_seed.zeroize();
        self.puk_seed.zeroize();
        self.self_token.zeroize();
    }
}

impl PendingYubiAccount {
    fn new_signup(
        alias: &str,
        username: &str,
        preparation: PendingYubiPreparation,
        device_name: String,
        email: String,
        invite: String,
        passphrase: Option<Vec<u8>>,
    ) -> Result<Self> {
        let mut subkey_seed = random_array()?;
        let mut puk_seed = random_array()?;
        let mut self_token: [u8; 17] = random_array()?;
        self_token[0] = 54;
        if subkey_seed == [0; 32] || puk_seed == [0; 32] {
            subkey_seed.zeroize();
            puk_seed.zeroize();
            self_token.zeroize();
            return Err(Error::Randomness);
        }
        Ok(Self {
            version: YUBI_RECORD_VERSION,
            alias: alias.to_owned(),
            username: username.to_owned(),
            locator: None,
            preparation: Some(preparation),
            subkey_seed,
            puk_seed: Some(puk_seed),
            self_token,
            purpose: PendingYubiPurpose::Signup {
                device_name,
                email,
                invite,
                passphrase,
            },
        })
    }

    fn new_provision(
        alias: &str,
        username: &str,
        preparation: PendingYubiPreparation,
        source_alias: String,
        device_name: String,
        serial: u64,
    ) -> Result<Self> {
        let mut pending = Self::new_signup(
            alias,
            username,
            preparation,
            String::new(),
            String::new(),
            String::new(),
            None,
        )?;
        pending.puk_seed.zeroize();
        pending.puk_seed = None;
        pending.purpose = PendingYubiPurpose::Provision {
            source_alias,
            device_name,
            serial,
        };
        Ok(pending)
    }

    fn locator(&self) -> Result<&YubiDeviceLocator> {
        self.locator.as_ref().ok_or(Error::InvalidAccount(
            "pending Yubi hardware preparation is incomplete",
        ))
    }

    fn signup_secrets(&self) -> Result<YubiAccountSecrets> {
        let puk_seed = self.puk_seed.ok_or(Error::InvalidAccount(
            "pending Yubi signup has no account recovery secret",
        ))?;
        Ok(YubiAccountSecrets::new(
            SecretSeed::new(self.subkey_seed),
            SecretSeed::new(puk_seed),
            self.self_token,
        ))
    }
}

pub struct LoadedYubiAccount {
    pub alias: String,
    pub username: String,
    pub uid: EntityId,
    pub locator: YubiDeviceLocator,
    pub subkey_id: EntityId,
    pub subkey_seed: SecretSeed,
    pub certificate_chain: Vec<Vec<u8>>,
    pub management_enrolled: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct YubiCardSummary {
    pub name: String,
    pub serial: u32,
}

/// A Yubi security-sync that also drove the federated responders.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct YubiFederationSyncReport {
    pub sync: SyncReport,
    pub federation: Vec<super::federation::FederationRefreshReport>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct YubiAccountReport {
    pub alias: String,
    pub username: String,
    pub yubi_id_hex: String,
    pub subkey_id_hex: String,
    pub user_chain_sequence: u64,
    pub management_enrolled: bool,
}

pub struct YubiSignupInput {
    pub alias: String,
    pub username: String,
    pub device_name: String,
    pub email: String,
    pub invite: String,
    pub passphrase: Option<Passphrase>,
    pub card: CardId,
    pub signing_slot: SlotId,
    pub pq_slot: SlotId,
    pub retry_configuration: Option<PinRetryConfiguration>,
}

pub struct YubiProvisionInput {
    pub source_alias: String,
    pub target_alias: String,
    pub device_name: String,
    pub serial: u64,
    pub card: CardId,
    pub signing_slot: SlotId,
    pub pq_slot: SlotId,
    pub retry_configuration: Option<PinRetryConfiguration>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct YubiLifecycleReport {
    pub alias: String,
    pub management_enrolled: bool,
    pub management_generation: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct YubiPinStatus {
    pub remaining: u8,
    pub blocked: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct YubiSubkeyRecoveryReport {
    pub alias: String,
    pub subkey_id_hex: String,
    pub certificate_count: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct YubiRevocationReport {
    pub alias: String,
    pub user_chain_sequence: u64,
    pub removed_local_credential: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct YubiAccountSummary {
    pub alias: String,
    pub state: YubiEnrollmentState,
    pub device_id_hex: Option<String>,
    pub card_serial: Option<u32>,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum YubiEnrollmentState {
    Pending,
    Complete,
}

impl From<PinRetries> for YubiPinStatus {
    fn from(value: PinRetries) -> Self {
        Self {
            remaining: value.remaining,
            blocked: value.blocked,
        }
    }
}

impl AccountVault<'_> {
    /// Lists both completed and resumable Yubi credential aliases so a
    /// frontend can discover lifecycle work after a process restart.
    pub fn yubi_aliases(&mut self) -> Result<Vec<String>> {
        let mut aliases = self
            .store
            .keys()?
            .into_iter()
            .filter_map(|key| yubi_alias_from_key(&key).map(str::to_owned))
            .collect::<Vec<_>>();
        aliases.sort();
        aliases.dedup();
        Ok(aliases)
    }

    /// Lists Yubi credential aliases without merging resumable enrollment
    /// records into completed credentials.
    pub fn yubi_accounts(&mut self) -> Result<Vec<YubiAccountSummary>> {
        let records = self
            .store
            .keys()?
            .into_iter()
            .filter_map(|key| {
                key.strip_prefix("pending-yubi.")
                    .map(|alias| (alias.to_owned(), YubiEnrollmentState::Pending))
                    .or_else(|| {
                        key.strip_prefix("yubi-account.")
                            .map(|alias| (alias.to_owned(), YubiEnrollmentState::Complete))
                    })
            })
            .collect::<Vec<_>>();
        let mut accounts = Vec::with_capacity(records.len());
        for (alias, state) in records {
            let (locator, card_serial) = match state {
                YubiEnrollmentState::Pending => {
                    let pending = self.pending_yubi(&alias)?;
                    let serial = pending
                        .locator
                        .as_ref()
                        .map(|locator| locator.card.serial)
                        .or_else(|| {
                            pending
                                .preparation
                                .as_ref()
                                .map(|preparation| preparation.card.serial)
                        });
                    (pending.locator.clone(), serial)
                }
                YubiEnrollmentState::Complete => {
                    let stored = self.stored_yubi(&alias)?;
                    (
                        Some(stored.locator.clone()),
                        Some(stored.locator.card.serial),
                    )
                }
            };
            // The authenticated device list uses this same EntityId encoding.
            let device_id_hex = locator.map(|locator| {
                let mut id = vec![foks_proto::ENTITY_YUBI];
                id.extend_from_slice(&locator.signing_public_key);
                hex(&id)
            });
            accounts.push(YubiAccountSummary {
                alias,
                state,
                device_id_hex,
                card_serial,
            });
        }
        accounts.sort_by(|left, right| {
            left.alias
                .cmp(&right.alias)
                .then_with(|| left.state.cmp(&right.state))
        });
        Ok(accounts)
    }

    pub(super) fn validate_pending_yubi_record(&mut self, alias: &str) -> Result<()> {
        drop(self.pending_yubi(alias)?);
        Ok(())
    }

    fn put_pending_yubi(&mut self, pending: &PendingYubiAccount) -> Result<()> {
        validate_pending_yubi(pending)?;
        let bytes = Zeroizing::new(serde_json::to_vec(pending)?);
        self.store.put(&pending_yubi_key(&pending.alias), &bytes)?;
        Ok(())
    }

    fn pending_yubi(&mut self, alias: &str) -> Result<PendingYubiAccount> {
        validate_name(alias)?;
        let bytes = self.store.get(&pending_yubi_key(alias)).map_err(|error| {
            if matches!(error, foks_keystore::Error::Missing) {
                Error::AccountMissing
            } else {
                Error::Keystore(error)
            }
        })?;
        let pending: PendingYubiAccount = serde_json::from_slice(&bytes)?;
        validate_pending_yubi(&pending)?;
        if pending.alias != alias {
            return Err(Error::InvalidAccount("pending Yubi alias changed"));
        }
        Ok(pending)
    }

    fn prepare_pending_yubi(
        &mut self,
        pending: &mut PendingYubiAccount,
        pin: &Pin,
        provider: &dyn YubiProvider,
    ) -> Result<PreparedYubiDevice> {
        loop {
            let Some(preparation) = pending.preparation.as_ref() else {
                let locator = pending.locator()?.clone();
                let device = provider.open(&locator, Some(pin))?;
                return Ok(PreparedYubiDevice {
                    entity_id: device.entity_id().clone(),
                    hepk: device.hepk().clone(),
                    locator,
                    device,
                });
            };
            let card = preparation.card.clone();
            let signing_slot = preparation.signing_slot;
            let pq_slot = preparation.pq_slot;
            let pin_policy = preparation.pin_policy;
            let touch_policy = preparation.touch_policy;
            match preparation.checkpoint {
                PendingYubiPreparationCheckpoint::Planned => {
                    let retry_requested = preparation.retry_configuration.is_some();
                    provider.prepare_preflight(
                        &card,
                        signing_slot,
                        pq_slot,
                        pin,
                        retry_requested,
                    )?;
                    pending
                        .preparation
                        .as_mut()
                        .expect("preparation is present")
                        .checkpoint = if retry_requested {
                        PendingYubiPreparationCheckpoint::RetryResetPending
                    } else {
                        PendingYubiPreparationCheckpoint::CredentialsReady
                    };
                    self.put_pending_yubi(pending)?;
                }
                PendingYubiPreparationCheckpoint::RetryResetPending
                | PendingYubiPreparationCheckpoint::PinRestorePending
                | PendingYubiPreparationCheckpoint::PukRestorePending => {
                    // Any credential command may have committed before its
                    // result was lost. Reissue SET PIN RETRIES to establish a
                    // known default baseline, then restore both intended
                    // credentials. Never infer completion from a 3/3 retry
                    // count or probe the intended and default PINs in turn.
                    let retry =
                        preparation
                            .retry_configuration
                            .as_ref()
                            .ok_or(Error::InvalidAccount(
                                "pending Yubi retry checkpoint has no policy",
                            ))?;
                    let pin_attempts = retry.pin_attempts;
                    let puk_attempts = retry.puk_attempts;
                    let puk = retry.puk()?;
                    provider.prepare_reset_retries(&card, pin_attempts, puk_attempts)?;
                    pending
                        .preparation
                        .as_mut()
                        .expect("preparation is present")
                        .checkpoint = PendingYubiPreparationCheckpoint::PinRestorePending;
                    self.put_pending_yubi(pending)?;
                    provider.prepare_restore_pin(&card, pin)?;
                    pending
                        .preparation
                        .as_mut()
                        .expect("preparation is present")
                        .checkpoint = PendingYubiPreparationCheckpoint::PukRestorePending;
                    self.put_pending_yubi(pending)?;
                    provider.prepare_restore_puk(&card, &puk)?;
                    pending
                        .preparation
                        .as_mut()
                        .expect("preparation is present")
                        .checkpoint = PendingYubiPreparationCheckpoint::CredentialsReady;
                    self.put_pending_yubi(pending)?;
                }
                PendingYubiPreparationCheckpoint::CredentialsReady => {
                    pending
                        .preparation
                        .as_mut()
                        .expect("preparation is present")
                        .checkpoint = PendingYubiPreparationCheckpoint::SigningKeyPending;
                    self.put_pending_yubi(pending)?;
                }
                PendingYubiPreparationCheckpoint::SigningKeyPending => {
                    let signing_public_key =
                        provider.prepare_key(&card, signing_slot, pin_policy, touch_policy)?;
                    pending
                        .preparation
                        .as_mut()
                        .expect("preparation is present")
                        .checkpoint =
                        PendingYubiPreparationCheckpoint::SigningKeyReady { signing_public_key };
                    self.put_pending_yubi(pending)?;
                }
                PendingYubiPreparationCheckpoint::SigningKeyReady { signing_public_key } => {
                    pending
                        .preparation
                        .as_mut()
                        .expect("preparation is present")
                        .checkpoint =
                        PendingYubiPreparationCheckpoint::PqKeyPending { signing_public_key };
                    self.put_pending_yubi(pending)?;
                }
                PendingYubiPreparationCheckpoint::PqKeyPending { signing_public_key } => {
                    let pq_public_key =
                        provider.prepare_key(&card, pq_slot, pin_policy, touch_policy)?;
                    pending
                        .preparation
                        .as_mut()
                        .expect("preparation is present")
                        .checkpoint = PendingYubiPreparationCheckpoint::KeysReady {
                        signing_public_key,
                        pq_public_key,
                    };
                    self.put_pending_yubi(pending)?;
                }
                PendingYubiPreparationCheckpoint::KeysReady {
                    signing_public_key,
                    pq_public_key,
                } => {
                    let locator = YubiDeviceLocator {
                        card,
                        signing_slot,
                        pq_slot,
                        signing_public_key,
                        pq_public_key,
                        pq_key_id: foks_crypto::yubi_pq_key_id(&pq_public_key)?,
                    };
                    let prepared = provider.finish_prepare(&locator, pin)?;
                    pending.locator = Some(locator);
                    pending.preparation = None;
                    self.put_pending_yubi(pending)?;
                    return Ok(prepared);
                }
            }
        }
    }

    pub(super) fn refresh_yubi_uid_labels(&mut self, uid: &EntityId, name: &str) -> Result<()> {
        let aliases = self
            .store
            .keys()?
            .into_iter()
            .filter_map(|k| k.strip_prefix("yubi-account.").map(str::to_owned))
            .collect::<Vec<_>>();
        for alias in aliases {
            let mut account = self.stored_yubi(&alias)?;
            if account.uid == uid.as_bytes() && account.username != name {
                account.username = name.into();
                self.put_stored_yubi(&account)?;
            }
        }
        Ok(())
    }
    fn stored_yubi(&mut self, alias: &str) -> Result<StoredYubiAccount> {
        validate_name(alias)?;
        let bytes = self.store.get(&yubi_account_key(alias)).map_err(|error| {
            if matches!(error, foks_keystore::Error::Missing) {
                Error::AccountMissing
            } else {
                Error::Keystore(error)
            }
        })?;
        let stored: StoredYubiAccount = serde_json::from_slice(&bytes)?;
        validate_stored_yubi(&stored, alias)?;
        Ok(stored)
    }

    fn put_stored_yubi(&mut self, stored: &StoredYubiAccount) -> Result<()> {
        validate_stored_yubi(stored, &stored.alias)?;
        let bytes = Zeroizing::new(serde_json::to_vec(stored)?);
        self.store.put(&yubi_account_key(&stored.alias), &bytes)?;
        Ok(())
    }

    pub fn yubi_account(&mut self, alias: &str) -> Result<LoadedYubiAccount> {
        let stored = self.stored_yubi(alias)?;
        Ok(LoadedYubiAccount {
            alias: stored.alias.clone(),
            username: stored.username.clone(),
            uid: EntityId::from_bytes(stored.uid.clone())?,
            locator: stored.locator.clone(),
            subkey_id: EntityId::from_bytes(stored.subkey_id.clone())?,
            subkey_seed: SecretSeed::new(stored.subkey_seed),
            certificate_chain: stored.certificate_chain.clone(),
            management_enrolled: stored.management_enrolled,
        })
    }

    #[cfg(test)]
    pub(super) fn yubi_management_generation(&mut self, alias: &str) -> Result<Option<u64>> {
        Ok(self.stored_yubi(alias)?.management_generation)
    }

    fn commit_created_yubi(
        &mut self,
        alias: &str,
        username: &str,
        locator: &YubiDeviceLocator,
        credential: &YubiCredential<'_>,
        management_refresh_source: Option<String>,
    ) -> Result<()> {
        let default_key = ManagementKey::default_piv();
        let next_key = ManagementKey::random()?;
        let stored = StoredYubiAccount {
            version: YUBI_RECORD_VERSION,
            alias: alias.to_owned(),
            username: username.to_owned(),
            uid: credential.uid.as_bytes().to_vec(),
            locator: locator.clone(),
            subkey_id: derive_subkey_id(&credential.subkey_seed)?.into_bytes(),
            subkey_seed: *credential.subkey_seed.as_bytes(),
            certificate_chain: credential.certificate_chain.clone(),
            management_key: Some(*default_key.expose()),
            pending_management_key: Some(*next_key.expose()),
            management_enrolled: false,
            management_generation: None,
            management_refresh_source,
        };
        self.put_stored_yubi(&stored)?;
        Ok(())
    }

    fn remove_pending_yubi(&mut self, alias: &str) -> Result<()> {
        self.store.remove(&pending_yubi_key(alias))?;
        Ok(())
    }
}

impl CheckedProfileSession<'_> {
    pub fn list_yubi_cards(&self, provider: &dyn YubiProvider) -> Result<Vec<YubiCardSummary>> {
        self.profile.require(Capability::DeviceAdministration)?;
        Ok(provider
            .cards()?
            .into_iter()
            .map(|card| YubiCardSummary {
                name: card.name,
                serial: card.serial,
            })
            .collect())
    }

    pub fn create_yubi_account(
        &self,
        input: YubiSignupInput,
        pin: Pin,
        provider: &dyn YubiProvider,
        vault: &mut AccountVault<'_>,
        master_key: &[u8; 32],
    ) -> Result<YubiAccountReport> {
        self.profile.require(Capability::Signup)?;
        self.profile.require(Capability::DeviceAdministration)?;
        if input.passphrase.is_some() {
            self.profile.require(Capability::Passphrases)?;
        }
        validate_name(&input.alias)?;
        if vault.contains(&input.alias)? {
            return Err(Error::AccountExists);
        }
        if foks_verify::normalize_username(input.username.as_bytes()).is_none()
            || input.device_name.len() > 256
            || input.email.len() > 320
        {
            return Err(Error::InvalidAccount(
                "Yubi signup fields are invalid before card preparation",
            ));
        }
        let invite_code = InviteCode::from_user_input(&input.invite, true)?;
        let host = self.pinned_host()?;
        self.client.check_invite_code(&host, &invite_code)?;
        let preparation = PendingYubiPreparation::new(
            input.card,
            input.signing_slot,
            input.pq_slot,
            input.retry_configuration,
        );
        let mut pending = PendingYubiAccount::new_signup(
            &input.alias,
            &input.username,
            preparation,
            input.device_name.clone(),
            input.email.clone(),
            input.invite.clone(),
            input
                .passphrase
                .as_ref()
                .map(|passphrase| passphrase.expose().to_vec()),
        )?;
        vault.put_pending_yubi(&pending)?;
        let prepared = vault.prepare_pending_yubi(&mut pending, &pin, provider)?;
        let mut mutations = EncryptedFileMutationStore::open(
            &self.paths.protected_mutations,
            derive_mutation_key(master_key),
        )?;
        let created = self.client.create_yubi_account(
            &host,
            prepared.device.as_ref(),
            YubiAccountRequest {
                username_utf8: input.username.clone(),
                device_name: input.device_name,
                invite_code,
                email: input.email,
                passphrase: input.passphrase,
                pq_hint: YubiSlotAndPqKeyId {
                    slot: u64::from(prepared.locator.pq_slot.get()),
                    id: prepared.locator.pq_key_id,
                },
            },
            pending.signup_secrets()?,
            &self.paths.soft_database,
            &mut mutations,
        )?;
        vault.commit_created_yubi(
            &input.alias,
            &input.username,
            &prepared.locator,
            &created.credential,
            None,
        )?;
        MutationCoordinator::new(&self.paths.hard_database, &mut mutations)
            .finalize(&created.operation_id)?;
        vault.remove_pending_yubi(&input.alias)?;
        drop(created);
        self.finish_management_rotation(
            &input.alias,
            provider,
            vault,
            Some(&pin),
            Some(master_key),
        )?;
        let loaded = vault.yubi_account(&input.alias)?;
        let sequence = self
            .client
            .authenticate_yubi_and_pin(&host, &loaded.credential(prepared.device.as_ref()))?
            .verified
            .chain_seqno();
        let stored = vault.stored_yubi(&input.alias)?;
        self.register_default_refresh_jobs_for(
            &EntityId::from_bytes(stored.uid.clone())?,
            now_microseconds()?,
        )?;
        Ok(YubiAccountReport {
            alias: input.alias,
            username: input.username,
            yubi_id_hex: hex(prepared.entity_id.as_bytes()),
            subkey_id_hex: hex(&stored.subkey_id),
            user_chain_sequence: sequence,
            management_enrolled: stored.management_enrolled,
        })
    }

    pub fn resume_yubi_account(
        &self,
        alias: &str,
        pin: Pin,
        provider: &dyn YubiProvider,
        vault: &mut AccountVault<'_>,
        master_key: &[u8; 32],
    ) -> Result<YubiAccountReport> {
        self.profile.require(Capability::DeviceAdministration)?;
        if vault
            .store
            .keys()?
            .iter()
            .any(|key| key == &yubi_account_key(alias))
        {
            let loaded = vault.yubi_account(alias)?;
            let device = provider.open(&loaded.locator, Some(&pin))?;
            let host = self.pinned_host()?;
            let mut mutations = EncryptedFileMutationStore::open(
                &self.paths.protected_mutations,
                derive_mutation_key(master_key),
            )?;
            for (kind, scope, subject) in [
                (
                    MutationKind::Signup,
                    device.entity_id().as_bytes(),
                    loaded.uid.as_bytes(),
                ),
                (
                    MutationKind::DeviceProvision,
                    loaded.uid.as_bytes(),
                    device.entity_id().as_bytes(),
                ),
            ] {
                if let Some(operation) = HardStateStore::open(&self.paths.hard_database)?
                    .latest_finalizable_mutation_for_binding(
                        host.host_id().as_bytes(),
                        kind,
                        scope,
                        subject,
                    )?
                {
                    MutationCoordinator::new(&self.paths.hard_database, &mut mutations)
                        .finalize(&operation.operation_id)?;
                }
            }
            let credential = loaded.credential(device.as_ref());
            let authenticated = self.client.authenticate_yubi_and_pin(&host, &credential)?;
            self.register_default_refresh_jobs_for(&loaded.uid, now_microseconds()?)?;
            let _ = vault.store.remove(&pending_yubi_key(alias))?;
            let responder_runs_in_finish = {
                let stored = vault.stored_yubi(alias)?;
                stored.pending_management_key.is_some() || !stored.management_enrolled
            };
            self.finish_management_rotation(alias, provider, vault, Some(&pin), Some(master_key))?;
            let stored = vault.stored_yubi(alias)?;
            if let Some(source) = stored.management_refresh_source.as_deref() {
                self.register_yubi_management_refresh(alias, source)?;
            }
            let authenticated = if responder_runs_in_finish {
                self.client.authenticate_yubi_and_pin(&host, &credential)?
            } else {
                self.run_unlocked_yubi_security_responders(
                    alias,
                    &host,
                    &credential,
                    authenticated,
                    vault,
                    master_key,
                )?
            };
            let stored = vault.stored_yubi(alias)?;
            return Ok(YubiAccountReport {
                alias: alias.to_owned(),
                username: stored.username.clone(),
                yubi_id_hex: hex(device.entity_id().as_bytes()),
                subkey_id_hex: hex(&stored.subkey_id),
                user_chain_sequence: authenticated.verified.chain_seqno(),
                management_enrolled: stored.management_enrolled,
            });
        }
        let mut pending = vault.pending_yubi(alias)?;
        if let PendingYubiPurpose::Signup { passphrase, .. } = &pending.purpose {
            self.profile.require(Capability::Signup)?;
            if passphrase.is_some() {
                self.profile.require(Capability::Passphrases)?;
            }
        }
        let prepared = vault.prepare_pending_yubi(&mut pending, &pin, provider)?;
        let locator = pending.locator()?.clone();
        let device = prepared.device;
        let host = self.pinned_host()?;
        let mut mutations = EncryptedFileMutationStore::open(
            &self.paths.protected_mutations,
            derive_mutation_key(master_key),
        )?;
        let refresh_source =
            match &pending.purpose {
                PendingYubiPurpose::Signup {
                    device_name,
                    email,
                    invite,
                    passphrase,
                } => {
                    let puk_seed = pending.puk_seed.ok_or(Error::InvalidAccount(
                        "pending Yubi signup has no account recovery secret",
                    ))?;
                    let mut uid =
                        derive_shared_verify_key(&SecretSeed::new(puk_seed), ENTITY_PUK_VERIFY)?
                            .into_bytes();
                    uid[0] = ENTITY_USER;
                    let uid = EntityId::from_bytes(uid)?;
                    let operation = HardStateStore::open(&self.paths.hard_database)?
                        .latest_mutation_for_binding(
                            host.host_id().as_bytes(),
                            MutationKind::Signup,
                            device.entity_id().as_bytes(),
                            uid.as_bytes(),
                        )?
                        .filter(|operation| operation.state != MutationState::Rejected);
                    let created = if let Some(operation) = operation {
                        let created = self.client.resume_yubi_account_with_pending(
                            &host,
                            device.as_ref(),
                            operation.operation_id,
                            &pending.username,
                            pending.signup_secrets()?,
                            &self.paths.soft_database,
                            &mut mutations,
                        )?;
                        if let Some(passphrase) = passphrase {
                            self.client.verify_passphrase_yubi(
                                &host,
                                &created.credential,
                                &Passphrase::new(passphrase)?,
                            )?;
                        }
                        created
                    } else {
                        self.client.create_yubi_account(
                            &host,
                            device.as_ref(),
                            YubiAccountRequest {
                                username_utf8: pending.username.clone(),
                                device_name: device_name.clone(),
                                invite_code: InviteCode::from_user_input(invite, true)?,
                                email: email.clone(),
                                passphrase: passphrase.as_ref().map(Passphrase::new).transpose()?,
                                pq_hint: YubiSlotAndPqKeyId {
                                    slot: u64::from(locator.pq_slot.get()),
                                    id: locator.pq_key_id,
                                },
                            },
                            pending.signup_secrets()?,
                            &self.paths.soft_database,
                            &mut mutations,
                        )?
                    };
                    vault.commit_created_yubi(
                        alias,
                        &pending.username,
                        &locator,
                        &created.credential,
                        None,
                    )?;
                    MutationCoordinator::new(&self.paths.hard_database, &mut mutations)
                        .finalize(&created.operation_id)?;
                    vault.remove_pending_yubi(alias)?;
                    None
                }
                PendingYubiPurpose::Provision {
                    source_alias,
                    device_name,
                    serial,
                } => {
                    let source = vault.account(source_alias)?;
                    if source.username != pending.username {
                        return Err(Error::InvalidAccount(
                            "pending Yubi provision source changed",
                        ));
                    }
                    let operation = HardStateStore::open(&self.paths.hard_database)?
                        .latest_mutation_for_binding(
                            host.host_id().as_bytes(),
                            MutationKind::DeviceProvision,
                            source.credential.uid.as_bytes(),
                            device.entity_id().as_bytes(),
                        )?
                        .filter(|operation| operation.state != MutationState::Rejected);
                    let provisioned =
                        if let Some(operation) = operation {
                            self.client.resume_yubi_device_provision(
                                &host,
                                &source.credential,
                                device.as_ref(),
                                operation.operation_id,
                                SecretSeed::new(pending.subkey_seed),
                                Role::OWNER,
                                &mut mutations,
                            )?
                        } else {
                            let current = self
                                .client
                                .authenticate_and_pin(&host, &source.credential)?;
                            if current.verified.devices().iter().any(|candidate| {
                                candidate.id.entity_type() == foks_proto::ENTITY_YUBI
                            }) {
                                return Err(Error::InvalidAccount(
                                    "revoke the existing hardware key before adding a replacement",
                                ));
                            }
                            self.client.provision_yubi_device(
                                &host,
                                &source.credential,
                                device.as_ref(),
                                YubiDeviceProvisionRequest {
                                    role: Role::OWNER,
                                    device_name: device_name.clone(),
                                    serial: *serial,
                                    pq_hint: YubiSlotAndPqKeyId {
                                        slot: u64::from(locator.pq_slot.get()),
                                        id: locator.pq_key_id,
                                    },
                                },
                                NewYubiDeviceSecrets::new(
                                    SecretSeed::new(pending.subkey_seed),
                                    pending.self_token,
                                ),
                                &mut mutations,
                            )?
                        };
                    vault.commit_created_yubi(
                        alias,
                        &pending.username,
                        &locator,
                        &provisioned.credential,
                        Some(source_alias.clone()),
                    )?;
                    MutationCoordinator::new(&self.paths.hard_database, &mut mutations)
                        .finalize(&provisioned.operation_id)?;
                    vault.remove_pending_yubi(alias)?;
                    Some(source_alias.clone())
                }
            };
        self.finish_management_rotation(alias, provider, vault, Some(&pin), Some(master_key))?;
        let loaded = vault.yubi_account(alias)?;
        let sequence = self
            .client
            .authenticate_yubi_and_pin(&host, &loaded.credential(device.as_ref()))?
            .verified
            .chain_seqno();
        if let Some(source) = refresh_source.as_deref() {
            self.register_yubi_management_refresh(alias, source)?;
        }
        let stored = vault.stored_yubi(alias)?;
        self.register_default_refresh_jobs_for(
            &EntityId::from_bytes(stored.uid.clone())?,
            now_microseconds()?,
        )?;
        Ok(YubiAccountReport {
            alias: alias.to_owned(),
            username: pending.username.clone(),
            yubi_id_hex: hex(device.entity_id().as_bytes()),
            subkey_id_hex: hex(&stored.subkey_id),
            user_chain_sequence: sequence,
            management_enrolled: stored.management_enrolled,
        })
    }

    pub fn provision_yubi_device(
        &self,
        input: YubiProvisionInput,
        pin: Pin,
        provider: &dyn YubiProvider,
        vault: &mut AccountVault<'_>,
        master_key: &[u8; 32],
    ) -> Result<YubiAccountReport> {
        self.profile.require(Capability::DeviceAdministration)?;
        validate_name(&input.target_alias)?;
        if vault.contains(&input.target_alias)? {
            return Err(Error::AccountExists);
        }
        if input.serial == 0 || input.device_name.len() > 256 {
            return Err(Error::InvalidAccount(
                "Yubi provision fields are invalid before card preparation",
            ));
        }
        let source = vault.account(&input.source_alias)?;
        let host = self.pinned_host()?;
        let current = self
            .client
            .authenticate_and_pin(&host, &source.credential)?;
        if current
            .verified
            .devices()
            .iter()
            .any(|device| device.id.entity_type() == foks_proto::ENTITY_YUBI)
        {
            return Err(Error::InvalidAccount(
                "revoke the existing hardware key before adding a replacement",
            ));
        }
        let preparation = PendingYubiPreparation::new(
            input.card,
            input.signing_slot,
            input.pq_slot,
            input.retry_configuration,
        );
        let mut pending = PendingYubiAccount::new_provision(
            &input.target_alias,
            &source.username,
            preparation,
            input.source_alias.clone(),
            input.device_name.clone(),
            input.serial,
        )?;
        vault.put_pending_yubi(&pending)?;
        let prepared = vault.prepare_pending_yubi(&mut pending, &pin, provider)?;
        let mut mutations = EncryptedFileMutationStore::open(
            &self.paths.protected_mutations,
            derive_mutation_key(master_key),
        )?;
        let provisioned = self.client.provision_yubi_device(
            &host,
            &source.credential,
            prepared.device.as_ref(),
            YubiDeviceProvisionRequest {
                role: Role::OWNER,
                device_name: input.device_name,
                serial: input.serial,
                pq_hint: YubiSlotAndPqKeyId {
                    slot: u64::from(prepared.locator.pq_slot.get()),
                    id: prepared.locator.pq_key_id,
                },
            },
            NewYubiDeviceSecrets::new(SecretSeed::new(pending.subkey_seed), pending.self_token),
            &mut mutations,
        )?;
        vault.commit_created_yubi(
            &input.target_alias,
            &source.username,
            &prepared.locator,
            &provisioned.credential,
            Some(input.source_alias.clone()),
        )?;
        MutationCoordinator::new(&self.paths.hard_database, &mut mutations)
            .finalize(&provisioned.operation_id)?;
        vault.remove_pending_yubi(&input.target_alias)?;
        drop(provisioned);
        self.finish_management_rotation(
            &input.target_alias,
            provider,
            vault,
            Some(&pin),
            Some(master_key),
        )?;
        let loaded = vault.yubi_account(&input.target_alias)?;
        let sequence = self
            .client
            .authenticate_yubi_and_pin(&host, &loaded.credential(prepared.device.as_ref()))?
            .verified
            .chain_seqno();
        self.register_yubi_management_refresh(&input.target_alias, &input.source_alias)?;
        let stored = vault.stored_yubi(&input.target_alias)?;
        Ok(YubiAccountReport {
            alias: input.target_alias,
            username: source.username,
            yubi_id_hex: hex(prepared.entity_id.as_bytes()),
            subkey_id_hex: hex(&stored.subkey_id),
            user_chain_sequence: sequence,
            management_enrolled: stored.management_enrolled,
        })
    }

    /// Opens one enrolled YubiKey and passes the unlocked credential into
    /// `operation` as a federation actor. The PIN and the hardware handle exist
    /// only for the duration of this call; neither is stored, journaled, or
    /// returned. Nesting two calls is the supported way to drive a refresh
    /// whose two sides need two different YubiKeys.
    pub fn with_unlocked_yubi<T>(
        &self,
        alias: &str,
        pin: &Pin,
        provider: &dyn YubiProvider,
        vault: &mut AccountVault<'_>,
        operation: impl FnOnce(UnlockedYubiActor<'_, '_>, &mut AccountVault<'_>) -> Result<T>,
    ) -> Result<T> {
        let loaded = vault.yubi_account(alias)?;
        let parent = provider.open(&loaded.locator, Some(pin))?;
        let credential = loaded.credential(parent.as_ref());
        operation(
            UnlockedYubiActor {
                profile: &self.profile.name,
                alias,
                credential: &credential,
            },
            vault,
        )
    }

    /// Synchronizes a Yubi account and additionally runs every federated
    /// security responder this unlocked device can drive. Remote sides that
    /// still need their own locked hardware are reported as deferred rather
    /// than failing the sync.
    #[allow(clippy::too_many_arguments)]
    pub fn sync_yubi_account_with_federation(
        &self,
        alias: &str,
        pin: Pin,
        provider: &dyn YubiProvider,
        vault: &mut AccountVault<'_>,
        registry: &ProfileRegistry,
        credentials: &ClientCredentials,
        master_key: &[u8; 32],
    ) -> Result<YubiFederationSyncReport> {
        // Check authority before touching hardware. A denied profile must not
        // consume a PIN attempt or make the user present a key for nothing.
        self.profile.require(Capability::UserSync)?;
        self.profile.require(Capability::Kv)?;
        self.with_unlocked_yubi(alias, &pin, provider, vault, |actor, vault| {
            let sync =
                self.sync_unlocked_yubi_account(actor.alias, actor.credential, vault, master_key)?;
            let federation = self.refresh_all_federated_security_with_unlocked_yubi(
                &[actor],
                vault,
                registry,
                credentials,
                master_key,
            )?;
            Ok(YubiFederationSyncReport { sync, federation })
        })
    }

    pub fn sync_yubi_account(
        &self,
        alias: &str,
        pin: Pin,
        provider: &dyn YubiProvider,
        vault: &mut AccountVault<'_>,
        master_key: &[u8; 32],
    ) -> Result<SyncReport> {
        self.profile.require(Capability::UserSync)?;
        self.profile.require(Capability::Kv)?;
        self.with_unlocked_yubi(alias, &pin, provider, vault, |actor, vault| {
            self.sync_unlocked_yubi_account(actor.alias, actor.credential, vault, master_key)
        })
    }

    fn sync_unlocked_yubi_account(
        &self,
        alias: &str,
        credential: &YubiCredential<'_>,
        vault: &mut AccountVault<'_>,
        master_key: &[u8; 32],
    ) -> Result<SyncReport> {
        self.profile.require(Capability::UserSync)?;
        self.profile.require(Capability::Kv)?;
        let host = self.pinned_host()?;
        let authenticated = self.client.authenticate_yubi_and_pin(&host, credential)?;
        let authenticated = self.run_unlocked_yubi_security_responders(
            alias,
            &host,
            credential,
            authenticated,
            vault,
            master_key,
        )?;
        let directories = self.client.sync_user_kv_yubi(
            &host,
            credential,
            &authenticated.verified,
            &authenticated.puks,
            &self.paths.soft_database,
        )?;
        self.refresh_verified_account_labels(&authenticated.verified, vault)?;
        Ok(SyncReport::from_tree(
            authenticated.verified.username(),
            authenticated.verified.chain_seqno(),
            &directories,
        ))
    }

    pub(super) fn run_unlocked_yubi_security_responders(
        &self,
        alias: &str,
        host: &foks_client::PinnedHost,
        credential: &YubiCredential<'_>,
        authenticated: foks_client::AuthenticatedUserOutcome,
        vault: &mut AccountVault<'_>,
        master_key: &[u8; 32],
    ) -> Result<foks_client::AuthenticatedUserOutcome> {
        let authenticated =
            self.refresh_yubi_user_security(host, credential, authenticated, master_key)?;
        if self
            .profile
            .require(Capability::DeviceAdministration)
            .is_ok()
        {
            self.refresh_unlocked_yubi_management_envelope(
                alias,
                host,
                credential,
                &authenticated,
                vault,
            )?;
        }
        self.refresh_yubi_team_chains(host, credential, authenticated, vault, master_key)
    }

    fn refresh_yubi_user_security(
        &self,
        host: &foks_client::PinnedHost,
        credential: &YubiCredential<'_>,
        authenticated: foks_client::AuthenticatedUserOutcome,
        master_key: &[u8; 32],
    ) -> Result<foks_client::AuthenticatedUserOutcome> {
        let mut mutations = EncryptedFileMutationStore::open(
            &self.paths.protected_mutations,
            derive_mutation_key(master_key),
        )?;
        let pending = HardStateStore::open(&self.paths.hard_database)?
            .pending_mutations(host.host_id().as_bytes())?
            .into_iter()
            .find(|operation| {
                operation.kind == MutationKind::PukRotation
                    && operation.scope_id == credential.uid.as_bytes()
            });
        let mut refreshed = authenticated;
        if let Some(operation) = pending {
            self.profile.require(Capability::DeviceAdministration)?;
            if self
                .client
                .journaled_yubi_puk_rotation_requires_passphrase_capability(
                    host,
                    credential,
                    operation.operation_id,
                    &mut mutations,
                )?
            {
                self.profile.require(Capability::Passphrases)?;
            }
            refreshed = if operation.subject_id == credential.parent.entity_id().as_bytes() {
                self.client.resume_yubi_puk_rotation_from_journal(
                    host,
                    credential,
                    operation.operation_id,
                    &mut mutations,
                )
            } else {
                self.client.reconcile_yubi_puk_rotation_from_journal(
                    host,
                    credential,
                    operation.operation_id,
                    &mut mutations,
                )
            }?;
            if HardStateStore::open(&self.paths.hard_database)?
                .mutation(&operation.operation_id)?
                .is_some_and(|operation| {
                    matches!(
                        operation.state,
                        foks_client_db::MutationState::Prepared
                            | foks_client_db::MutationState::Submitting
                            | foks_client_db::MutationState::SubmissionUnknown
                            | foks_client_db::MutationState::RemoteVerified
                    )
                })
            {
                return Err(Error::InvalidAccount(
                    "journaled PUK rotation remains ambiguous",
                ));
            }
        }
        if let Some(upper) = refreshed
            .verified
            .stale_shared_key_roles()
            .iter()
            .next_back()
            .copied()
        {
            let expected_version = refreshed
                .verified
                .chain_seqno()
                .checked_add(1)
                .ok_or(Error::InvalidAccount("user chain sequence overflow"))?;
            if HardStateStore::open(&self.paths.hard_database)?
                .pending_mutations(host.host_id().as_bytes())?
                .into_iter()
                .any(|operation| {
                    operation.scope_id == credential.uid.as_bytes()
                        && operation.expected_version == Some(expected_version)
                        && matches!(
                            operation.kind,
                            MutationKind::DeviceProvision
                                | MutationKind::DeviceRevoke
                                | MutationKind::PukRotation
                        )
                })
            {
                return Err(Error::InvalidAccount(
                    "an active user-chain mutation reserves the stale-PUK rotation position",
                ));
            }
            self.profile.require(Capability::DeviceAdministration)?;
            let rotations = refreshed
                .verified
                .shared_keys()
                .iter()
                .filter(|key| key.role <= upper)
                .map(|key| {
                    let role_history = self.client.load_puks_for_role_yubi(
                        host,
                        credential,
                        &refreshed.verified,
                        key.role,
                    )?;
                    let previous = role_history
                        .into_iter()
                        .find(|private| {
                            private.role == key.role && private.generation == key.generation
                        })
                        .ok_or(Error::InvalidAccount(
                            "current stale PUK material is unavailable",
                        ))?;
                    Ok(foks_client::UserPukRotation {
                        role: key.role,
                        previous_generation: key.generation,
                        previous_seed: previous.seed,
                        new_seed: SecretSeed::new(random_array()?),
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            let rotates_owner = rotations
                .iter()
                .any(|rotation| rotation.role == Role::OWNER);
            let no_passphrase = if rotates_owner {
                self.profile.require(Capability::Passphrases)?;
                (!self
                    .client
                    .passphrase_is_configured_yubi(host, credential)?)
                .then_some(foks_client::NoPassphraseConfigured)
            } else {
                None
            };
            refreshed = self.client.rotate_yubi_puks(
                host,
                credential,
                &rotations,
                no_passphrase,
                &mut mutations,
            )?;
        }
        if self.profile.require(Capability::Passphrases).is_ok() {
            self.client
                .refresh_passphrase_for_current_puk_yubi(host, credential, &refreshed)?;
        }
        Ok(refreshed)
    }

    fn refresh_unlocked_yubi_management_envelope(
        &self,
        alias: &str,
        host: &foks_client::PinnedHost,
        credential: &YubiCredential<'_>,
        authenticated: &foks_client::AuthenticatedUserOutcome,
        vault: &mut AccountVault<'_>,
    ) -> Result<()> {
        let mut stored = vault.stored_yubi(alias)?;
        if !stored.management_enrolled {
            return Ok(());
        }
        stored.require_completed_management_rotation()?;
        let current = authenticated
            .puks
            .iter()
            .filter(|puk| puk.role == Role::OWNER)
            .max_by_key(|puk| puk.generation)
            .ok_or(Error::InvalidAccount("current owner PUK is unavailable"))?;
        self.client.refresh_yubi_management_key_yubi(
            host,
            credential,
            current,
            &authenticated.puks,
        )?;
        stored.management_generation = Some(current.generation);
        vault.put_stored_yubi(&stored)
    }

    pub fn set_yubi_passphrase(
        &self,
        alias: &str,
        pin: Pin,
        passphrase: Passphrase,
        provider: &dyn YubiProvider,
        vault: &mut AccountVault<'_>,
        master_key: &[u8; 32],
    ) -> Result<PassphraseReport> {
        self.profile.require(Capability::Passphrases)?;
        let loaded = vault.yubi_account(alias)?;
        let parent = provider.open(&loaded.locator, Some(&pin))?;
        let credential = loaded.credential(parent.as_ref());
        let host = self.pinned_host()?;
        let authenticated = self.client.authenticate_yubi_and_pin(&host, &credential)?;
        self.run_unlocked_yubi_security_responders(
            alias,
            &host,
            &credential,
            authenticated,
            vault,
            master_key,
        )?;
        // The write already read the committed parcel back and validated it
        // against the argument it signed, so the confirmation here is the
        // server login assertion alone.
        let (metadata, verification) =
            self.client
                .set_passphrase_yubi_verified(&host, &credential, &passphrase)?;
        PassphraseReport::from_verified(metadata, verification)
    }

    pub fn change_yubi_passphrase(
        &self,
        alias: &str,
        pin: Pin,
        passphrase: Passphrase,
        provider: &dyn YubiProvider,
        vault: &mut AccountVault<'_>,
        master_key: &[u8; 32],
    ) -> Result<PassphraseReport> {
        self.profile.require(Capability::Passphrases)?;
        let loaded = vault.yubi_account(alias)?;
        let parent = provider.open(&loaded.locator, Some(&pin))?;
        let credential = loaded.credential(parent.as_ref());
        let host = self.pinned_host()?;
        let authenticated = self.client.authenticate_yubi_and_pin(&host, &credential)?;
        self.run_unlocked_yubi_security_responders(
            alias,
            &host,
            &credential,
            authenticated,
            vault,
            master_key,
        )?;
        let (metadata, verification) =
            self.client
                .change_passphrase_yubi_verified(&host, &credential, &passphrase)?;
        PassphraseReport::from_verified(metadata, verification)
    }

    /// Verifies a passphrase and reports what the server holds. This is a
    /// read: it does not run the unlocked security responders, so it matches
    /// the software surface, which also verifies once and sweeps nothing. The
    /// rollback guard that used to follow existed only to bracket the
    /// interposed sweep, so it goes with it; a command that does mutate still
    /// runs the responders itself, and the periodic account sync runs them
    /// whether or not a passphrase is ever verified. Taking no vault master
    /// key is the visible form of that: nothing here can rewrap one.
    pub fn verify_yubi_passphrase(
        &self,
        alias: &str,
        pin: Pin,
        passphrase: Passphrase,
        provider: &dyn YubiProvider,
        vault: &mut AccountVault<'_>,
    ) -> Result<PassphraseReport> {
        self.profile.require(Capability::Passphrases)?;
        let loaded = vault.yubi_account(alias)?;
        let parent = provider.open(&loaded.locator, Some(&pin))?;
        let credential = loaded.credential(parent.as_ref());
        let host = self.pinned_host()?;
        let verification = self
            .client
            .verify_passphrase_yubi(&host, &credential, &passphrase)?;
        Ok(PassphraseReport {
            generation: verification.generation,
            stretch_version: "v1",
            verified: true,
        })
    }

    pub fn recover_yubi_subkey(
        &self,
        alias: &str,
        pin: Pin,
        provider: &dyn YubiProvider,
        vault: &mut AccountVault<'_>,
        master_key: &[u8; 32],
    ) -> Result<YubiSubkeyRecoveryReport> {
        self.profile.require(Capability::Recovery)?;
        let mut stored = vault.stored_yubi(alias)?;
        let parent = provider.open(&stored.locator, Some(&pin))?;
        let expected = EntityId::from_bytes(stored.subkey_id.clone())?;
        let host = self.pinned_host()?;
        let recovered = self.client.recover_yubi_credential(
            &host,
            EntityId::from_bytes(stored.uid.clone())?,
            &expected,
            parent.as_ref(),
        )?;
        if derive_subkey_id(&recovered.subkey_seed)? != expected {
            return Err(Error::InvalidAccount(
                "recovered Yubi subkey does not match the durable binding",
            ));
        }
        stored.subkey_seed = *recovered.subkey_seed.as_bytes();
        stored.certificate_chain = recovered.certificate_chain.clone();
        let certificate_count = stored.certificate_chain.len();
        vault.put_stored_yubi(&stored)?;
        let authenticated = self.client.authenticate_yubi_and_pin(&host, &recovered)?;
        self.run_unlocked_yubi_security_responders(
            alias,
            &host,
            &recovered,
            authenticated,
            vault,
            master_key,
        )?;
        Ok(YubiSubkeyRecoveryReport {
            alias: alias.to_owned(),
            subkey_id_hex: hex(expected.as_bytes()),
            certificate_count,
        })
    }

    pub fn revoke_yubi_device(
        &self,
        software_alias: &str,
        yubi_alias: &str,
        vault: &mut AccountVault<'_>,
        master_key: &[u8; 32],
    ) -> Result<YubiRevocationReport> {
        self.profile.require(Capability::DeviceAdministration)?;
        let software = vault.account(software_alias)?;
        let stored = vault.stored_yubi(yubi_alias)?;
        if software.credential.uid.as_bytes() != stored.uid {
            return Err(Error::InvalidAccount(
                "software and Yubi aliases belong to different users",
            ));
        }
        let target = EntityId::from_bytes({
            let mut id = Vec::with_capacity(34);
            id.push(foks_proto::ENTITY_YUBI);
            id.extend_from_slice(&stored.locator.signing_public_key);
            id
        })?;
        let host = self.pinned_host()?;
        let authenticated = self
            .client
            .authenticate_and_pin(&host, &software.credential)?;
        let target_role = authenticated
            .verified
            .devices()
            .iter()
            .find(|device| device.id == target)
            .map(|device| device.role);
        let Some(target_role) = target_role else {
            self.cleanup_revoked_yubi(yubi_alias, &stored, vault)?;
            return Ok(YubiRevocationReport {
                alias: yubi_alias.to_owned(),
                user_chain_sequence: authenticated.verified.chain_seqno(),
                removed_local_credential: true,
            });
        };
        let (rotations, no_passphrase) = self.software_revocation_material(
            &host,
            &software.credential,
            &authenticated,
            target_role,
        )?;
        let mut mutations = EncryptedFileMutationStore::open(
            &self.paths.protected_mutations,
            derive_mutation_key(master_key),
        )?;
        let revoked = self.client.revoke_user_credential_with_software_device(
            &host,
            &software.credential,
            &target,
            &rotations,
            no_passphrase,
            &mut mutations,
        )?;
        self.cleanup_revoked_yubi(yubi_alias, &stored, vault)?;
        Ok(YubiRevocationReport {
            alias: yubi_alias.to_owned(),
            user_chain_sequence: revoked.verified.chain_seqno(),
            removed_local_credential: true,
        })
    }

    fn cleanup_revoked_yubi(
        &self,
        yubi_alias: &str,
        stored: &StoredYubiAccount,
        vault: &mut AccountVault<'_>,
    ) -> Result<()> {
        if let Some(source) = stored.management_refresh_source.as_deref() {
            let job_id = yubi_refresh_job_id(yubi_alias, source)?;
            let _ = foks_client::FoksScheduler::new(
                &self.paths.hard_database,
                foks_client::SchedulerConfig::default(),
            )?
            .unregister(&job_id)?;
        }
        let _ = vault.store.remove(&yubi_account_key(yubi_alias))?;
        Ok(())
    }

    pub fn yubi_pin_status(
        &self,
        alias: &str,
        provider: &dyn YubiProvider,
        vault: &mut AccountVault<'_>,
    ) -> Result<YubiPinStatus> {
        self.profile.require(Capability::DeviceAdministration)?;
        let loaded = vault.yubi_account(alias)?;
        Ok(provider.open_admin(&loaded.locator)?.pin_retries()?.into())
    }

    pub fn change_yubi_pin(
        &self,
        alias: &str,
        old_pin: Pin,
        new_pin: Pin,
        provider: &dyn YubiProvider,
        vault: &mut AccountVault<'_>,
    ) -> Result<YubiPinStatus> {
        self.profile.require(Capability::DeviceAdministration)?;
        let loaded = vault.yubi_account(alias)?;
        let admin = provider.open_admin(&loaded.locator)?;
        admin.change_pin(&old_pin, &new_pin)?;
        Ok(admin.pin_retries()?.into())
    }

    pub fn change_yubi_puk(
        &self,
        alias: &str,
        old_puk: Pin,
        new_puk: Pin,
        provider: &dyn YubiProvider,
        vault: &mut AccountVault<'_>,
    ) -> Result<()> {
        self.profile.require(Capability::DeviceAdministration)?;
        let loaded = vault.yubi_account(alias)?;
        provider
            .open_admin(&loaded.locator)?
            .change_puk(&old_puk, &new_puk)?;
        Ok(())
    }

    pub fn unblock_yubi_pin(
        &self,
        alias: &str,
        puk: Pin,
        new_pin: Pin,
        provider: &dyn YubiProvider,
        vault: &mut AccountVault<'_>,
    ) -> Result<YubiPinStatus> {
        self.profile.require(Capability::DeviceAdministration)?;
        let loaded = vault.yubi_account(alias)?;
        let admin = provider.open_admin(&loaded.locator)?;
        admin.unblock_pin(&puk, &new_pin)?;
        Ok(admin.pin_retries()?.into())
    }

    pub fn rotate_yubi_management_key(
        &self,
        alias: &str,
        pin: Pin,
        provider: &dyn YubiProvider,
        vault: &mut AccountVault<'_>,
        master_key: &[u8; 32],
    ) -> Result<YubiLifecycleReport> {
        self.profile.require(Capability::DeviceAdministration)?;
        let mut stored = vault.stored_yubi(alias)?;
        if stored.pending_management_key.is_none() {
            stored.pending_management_key = Some(*ManagementKey::random()?.expose());
            stored.management_enrolled = false;
            stored.management_generation = None;
            vault.put_stored_yubi(&stored)?;
        }
        self.finish_management_rotation(alias, provider, vault, Some(&pin), Some(master_key))
    }

    pub fn resume_yubi_management_key(
        &self,
        alias: &str,
        pin: Option<Pin>,
        provider: &dyn YubiProvider,
        vault: &mut AccountVault<'_>,
        master_key: &[u8; 32],
    ) -> Result<YubiLifecycleReport> {
        self.profile.require(Capability::DeviceAdministration)?;
        self.finish_management_rotation(alias, provider, vault, pin.as_ref(), Some(master_key))
    }

    fn finish_management_rotation(
        &self,
        alias: &str,
        provider: &dyn YubiProvider,
        vault: &mut AccountVault<'_>,
        pin: Option<&Pin>,
        master_key: Option<&[u8; 32]>,
    ) -> Result<YubiLifecycleReport> {
        let mut stored = vault.stored_yubi(alias)?;
        if let Some(next) = stored.pending_management_key {
            let current = stored.management_key.ok_or(Error::InvalidAccount(
                "current Yubi management key is unavailable",
            ))?;
            let admin = provider.open_admin(&stored.locator)?;
            let next_key = ManagementKey::from_bytes(next);
            if admin
                .replace_management_key(&ManagementKey::from_bytes(current), &next_key, false)
                .is_err()
            {
                // Crash recovery: if the first replacement committed, proving
                // and setting the same key is an idempotent completion.
                admin.replace_management_key(&next_key, &next_key, false)?;
            }
        }
        if stored.pending_management_key.is_some() || !stored.management_enrolled {
            let pin = pin.ok_or(Error::InvalidAccount(
                "PIN is required to publish the Yubi management-key envelope",
            ))?;
            let device = provider.open(&stored.locator, Some(pin))?;
            let credential = YubiCredential {
                uid: EntityId::from_bytes(stored.uid.clone())?,
                parent: device.as_ref(),
                subkey_seed: SecretSeed::new(stored.subkey_seed),
                certificate_chain: stored.certificate_chain.clone(),
            };
            let host = self.pinned_host()?;
            let mut authenticated = self.client.authenticate_yubi_and_pin(&host, &credential)?;
            if let Some(master_key) = master_key {
                // Close stale-PUK and PPE gaps before encrypting a newly
                // generated management key to the owner role.
                authenticated =
                    self.refresh_yubi_user_security(&host, &credential, authenticated, master_key)?;
            }
            let owner = authenticated
                .puks
                .iter()
                .find(|puk| puk.role == Role::OWNER)
                .ok_or(Error::InvalidAccount("owner PUK is unavailable"))?;
            let key = stored.management_key_for_publication()?;
            let envelope = encrypt_yubi_management_key(
                owner,
                device.entity_id(),
                YubiCardId {
                    name: stored.locator.card.name.as_bytes().to_vec(),
                    serial: u64::from(stored.locator.card.serial),
                },
                u64::from(stored.locator.signing_slot.get()),
                &key,
            )?;
            self.client
                .put_yubi_management_key_yubi(&host, &credential, &envelope)?;
            // The durable record remains old-current/pending-new until the
            // server has acknowledged the new envelope. A retry can therefore
            // prove either side of the card replacement and republish safely.
            stored.complete_management_publication(owner.generation);
            vault.put_stored_yubi(&stored)?;
            if let Some(master_key) = master_key {
                self.refresh_yubi_team_chains(
                    &host,
                    &credential,
                    authenticated,
                    vault,
                    master_key,
                )?;
                stored = vault.stored_yubi(alias)?;
            }
        }
        Ok(YubiLifecycleReport {
            alias: alias.to_owned(),
            management_enrolled: stored.management_enrolled,
            management_generation: stored.management_generation,
        })
    }

    pub fn recover_yubi_management_key(
        &self,
        yubi_alias: &str,
        software_alias: &str,
        vault: &mut AccountVault<'_>,
    ) -> Result<YubiLifecycleReport> {
        self.profile.require(Capability::DeviceAdministration)?;
        self.profile.require(Capability::Recovery)?;
        let mut stored = vault.stored_yubi(yubi_alias)?;
        stored.require_completed_management_rotation()?;
        let software = vault.account(software_alias)?;
        if software.credential.uid.as_bytes() != stored.uid {
            return Err(Error::InvalidAccount(
                "software and Yubi aliases belong to different users",
            ));
        }
        let host = self.pinned_host()?;
        let authenticated = self
            .client
            .authenticate_and_pin(&host, &software.credential)?;
        let parent = EntityId::from_bytes({
            let mut id = Vec::with_capacity(34);
            id.push(foks_proto::ENTITY_YUBI);
            id.extend_from_slice(&stored.locator.signing_public_key);
            id
        })?;
        let envelope = self
            .client
            .get_yubi_management_key(&host, &software.credential, &parent)?;
        let recovered = decrypt_yubi_management_key(&envelope, &authenticated.puks)?;
        if recovered.card.name != stored.locator.card.name.as_bytes()
            || recovered.card.serial != u64::from(stored.locator.card.serial)
            || recovered.slot != u64::from(stored.locator.signing_slot.get())
        {
            return Err(Error::InvalidAccount(
                "recovered management key is bound to another card or slot",
            ));
        }
        stored.management_key = Some(*recovered.management_key);
        stored.pending_management_key = None;
        stored.management_enrolled = true;
        stored.management_generation = Some(recovered.puk_generation);
        vault.put_stored_yubi(&stored)?;
        Ok(YubiLifecycleReport {
            alias: yubi_alias.to_owned(),
            management_enrolled: true,
            management_generation: Some(recovered.puk_generation),
        })
    }

    pub fn register_yubi_management_refresh(
        &self,
        yubi_alias: &str,
        software_alias: &str,
    ) -> Result<[u8; 16]> {
        self.profile.require(Capability::DeviceAdministration)?;
        validate_name(yubi_alias)?;
        validate_name(software_alias)?;
        let scope = serde_json::to_vec(&YubiRefreshScope {
            yubi_alias: yubi_alias.to_owned(),
            software_alias: software_alias.to_owned(),
        })?;
        let host = self.pinned_host()?;
        let job_id = yubi_refresh_job_id(yubi_alias, software_alias)?;
        let now = now_microseconds()?;
        foks_client::FoksScheduler::new(
            &self.paths.hard_database,
            foks_client::SchedulerConfig::default(),
        )?
        .register(foks_client::ScheduledJobRegistration {
            job_id,
            kind: ScheduledJobKind::YubiManagementRefresh,
            host_id: host.host_id().as_bytes().to_vec(),
            scope_id: scope,
            interval_micros: 24 * 60 * 60 * 1_000_000,
            first_run_at: now
                .checked_add(24 * 60 * 60 * 1_000_000)
                .ok_or(Error::InvalidConfig("Yubi refresh time overflow"))?,
            registered_at: now,
        })?;
        Ok(job_id)
    }

    pub(super) fn refresh_yubi_management_envelope(
        &self,
        yubi_alias: &str,
        software_alias: &str,
        vault: &mut AccountVault<'_>,
    ) -> Result<()> {
        let mut stored = vault.stored_yubi(yubi_alias)?;
        stored.require_completed_management_rotation()?;
        let software = vault.account(software_alias)?;
        if software.credential.uid.as_bytes() != stored.uid {
            return Err(Error::InvalidAccount(
                "scheduled Yubi refresh aliases belong to different users",
            ));
        }
        let host = self.pinned_host()?;
        let authenticated = self
            .client
            .authenticate_and_pin(&host, &software.credential)?;
        let current = authenticated
            .puks
            .iter()
            .filter(|puk| puk.role == Role::OWNER)
            .max_by_key(|puk| puk.generation)
            .ok_or(Error::InvalidAccount("current owner PUK is unavailable"))?;
        let parent = EntityId::from_bytes({
            let mut id = Vec::with_capacity(34);
            id.push(foks_proto::ENTITY_YUBI);
            id.extend_from_slice(&stored.locator.signing_public_key);
            id
        })?;
        self.client.refresh_yubi_management_key(
            &host,
            &software.credential,
            &parent,
            current,
            &authenticated.puks,
        )?;
        stored.management_enrolled = true;
        stored.management_generation = Some(current.generation);
        vault.put_stored_yubi(&stored)
    }
}

impl LoadedYubiAccount {
    /// Binds this durable record to an already-opened hardware parent. The
    /// caller owns the device handle, so the credential can be assembled
    /// outside a checked profile session and used as the far side of a
    /// two-key federation refresh.
    pub fn credential<'a>(&self, parent: &'a dyn YubiDevice) -> YubiCredential<'a> {
        YubiCredential {
            uid: self.uid.clone(),
            parent,
            subkey_seed: SecretSeed::new(*self.subkey_seed.as_bytes()),
            certificate_chain: self.certificate_chain.clone(),
        }
    }
}

fn validate_pending_yubi(pending: &PendingYubiAccount) -> Result<()> {
    if pending.version != YUBI_RECORD_VERSION
        || pending.self_token[0] != 54
        || pending.subkey_seed == [0; 32]
        || pending.puk_seed == Some([0; 32])
    {
        return Err(Error::InvalidAccount("pending Yubi record is invalid"));
    }
    validate_name(&pending.alias)?;
    if foks_verify::normalize_username(pending.username.as_bytes()).is_none() {
        return Err(Error::InvalidAccount("pending Yubi username is invalid"));
    }
    match &pending.purpose {
        PendingYubiPurpose::Signup {
            device_name,
            email,
            invite,
            passphrase,
        } => {
            if pending.puk_seed.is_none()
                || device_name.len() > 256
                || email.len() > 320
                || InviteCode::from_user_input(invite, true).is_err()
                || passphrase
                    .as_ref()
                    .is_some_and(|passphrase| Passphrase::new(passphrase).is_err())
            {
                return Err(Error::InvalidAccount(
                    "pending Yubi signup request is invalid",
                ));
            }
        }
        PendingYubiPurpose::Provision {
            source_alias,
            device_name,
            serial,
        } => {
            if pending.puk_seed.is_some()
                || validate_name(source_alias).is_err()
                || device_name.len() > 256
                || *serial == 0
            {
                return Err(Error::InvalidAccount(
                    "pending Yubi provision request is invalid",
                ));
            }
        }
    }
    match (&pending.locator, &pending.preparation) {
        (Some(locator), None) => validate_locator(locator),
        (None, Some(preparation)) => validate_pending_yubi_preparation(preparation),
        _ => Err(Error::InvalidAccount(
            "pending Yubi preparation state is inconsistent",
        )),
    }
}

fn validate_pending_yubi_preparation(preparation: &PendingYubiPreparation) -> Result<()> {
    if preparation.card.serial == 0
        || preparation.card.name.is_empty()
        || preparation.card.name.len() > 255
        || preparation.card.name.as_bytes().contains(&0)
        || preparation.signing_slot == preparation.pq_slot
    {
        return Err(Error::InvalidAccount("pending Yubi preparation is invalid"));
    }
    if let Some(retry) = &preparation.retry_configuration {
        if !(1..=15).contains(&retry.pin_attempts)
            || !(1..=15).contains(&retry.puk_attempts)
            || std::str::from_utf8(&retry.puk)
                .ok()
                .and_then(|puk| Pin::new(puk).ok())
                .is_none()
        {
            return Err(Error::InvalidAccount(
                "pending Yubi retry configuration is invalid",
            ));
        }
    } else if matches!(
        preparation.checkpoint,
        PendingYubiPreparationCheckpoint::RetryResetPending
            | PendingYubiPreparationCheckpoint::PinRestorePending
            | PendingYubiPreparationCheckpoint::PukRestorePending
    ) {
        return Err(Error::InvalidAccount(
            "pending Yubi retry checkpoint has no policy",
        ));
    }
    Ok(())
}

fn validate_stored_yubi(stored: &StoredYubiAccount, alias: &str) -> Result<()> {
    if stored.version != YUBI_RECORD_VERSION || stored.alias != alias {
        return Err(Error::InvalidAccount(
            "stored Yubi record version or alias does not match",
        ));
    }
    validate_name(alias)?;
    if foks_verify::normalize_username(stored.username.as_bytes()).is_none() {
        return Err(Error::InvalidAccount("stored Yubi username is invalid"));
    }
    EntityId::from_bytes(stored.uid.clone())?.require_type(ENTITY_USER)?;
    EntityId::from_bytes(stored.subkey_id.clone())?.require_type(foks_proto::ENTITY_SUBKEY)?;
    if derive_subkey_id(&SecretSeed::new(stored.subkey_seed))?.as_bytes() != stored.subkey_id {
        return Err(Error::InvalidAccount(
            "stored Yubi subkey seed does not match subkey ID",
        ));
    }
    validate_certificates(&stored.certificate_chain)?;
    validate_locator(&stored.locator)?;
    if stored.pending_management_key.is_some() && stored.management_key.is_none() {
        return Err(Error::InvalidAccount(
            "pending Yubi management key has no current key",
        ));
    }
    if stored.pending_management_key.is_some()
        && (stored.management_enrolled || stored.management_generation.is_some())
    {
        return Err(Error::InvalidAccount(
            "pending Yubi management key is marked as enrolled",
        ));
    }
    if stored.management_enrolled != stored.management_generation.is_some() {
        return Err(Error::InvalidAccount(
            "Yubi management envelope generation is inconsistent",
        ));
    }
    if let Some(source) = stored.management_refresh_source.as_deref() {
        validate_name(source)?;
    }
    Ok(())
}

fn yubi_refresh_job_id(yubi_alias: &str, software_alias: &str) -> Result<[u8; 16]> {
    validate_name(yubi_alias)?;
    validate_name(software_alias)?;
    let scope = serde_json::to_vec(&YubiRefreshScope {
        yubi_alias: yubi_alias.to_owned(),
        software_alias: software_alias.to_owned(),
    })?;
    let hash = prefixed_hash(YUBI_REFRESH_JOB_TYPE_ID, &scope);
    Ok(hash[..16]
        .try_into()
        .expect("a hash prefix is exactly sixteen bytes"))
}

fn validate_locator(locator: &YubiDeviceLocator) -> Result<()> {
    if locator.card.serial == 0
        || locator.card.name.is_empty()
        || locator.card.name.len() > 255
        || locator.card.name.as_bytes().contains(&0)
        || locator.signing_slot == locator.pq_slot
        || foks_crypto::yubi_pq_key_id(&locator.pq_public_key)? != locator.pq_key_id
    {
        return Err(Error::InvalidAccount("Yubi locator is invalid"));
    }
    Ok(())
}

impl AccountVault<'_> {
    pub(super) fn pending_yubi_sso_intent(
        &mut self,
        alias: &str,
    ) -> Result<foks_client::SsoIntent> {
        let pending = self.pending_yubi(alias)?;
        if !matches!(pending.purpose, PendingYubiPurpose::Signup { .. }) {
            return Err(Error::InvalidAccount("hardware operation is not a signup"));
        }
        let seed = pending
            .puk_seed
            .ok_or(Error::InvalidAccount("hardware signup seed is missing"))?;
        let mut uid =
            derive_shared_verify_key(&SecretSeed::new(seed), ENTITY_PUK_VERIFY)?.into_bytes();
        uid[0] = ENTITY_USER;
        let device = EntityId::from_bytes(
            [
                vec![foks_proto::ENTITY_YUBI],
                pending.locator()?.signing_public_key.to_vec(),
            ]
            .concat(),
        )?;
        Ok(foks_client::SsoIntent {
            uid: EntityId::from_bytes(uid)?,
            device,
            purpose: foks_proto::SsoPurpose::Signup,
        })
    }
}
impl CheckedProfileSession<'_> {
    pub fn begin_yubi_sso_signup(
        &self,
        input: YubiSignupInput,
        pin: Pin,
        provider: &dyn YubiProvider,
        vault: &mut AccountVault<'_>,
        master: &[u8; 32],
        http: &foks_oidc::ProviderHttp,
    ) -> Result<crate::SsoReport> {
        self.profile.require(Capability::Signup)?;
        self.profile.require(Capability::DeviceAdministration)?;
        validate_name(&input.alias)?;
        if input.device_name.is_empty() || input.device_name.len() > 256 {
            return Err(Error::InvalidAccount("invalid hardware device name"));
        }
        if input.passphrase.is_some() {
            self.profile.require(Capability::Passphrases)?;
        }
        // Check that this host requires OIDC before changing a hardware slot.
        if self
            .client
            .registration_server_config(&self.pinned_host()?)?
            .sso
            .is_none_or(|c| c.active != foks_proto::SsoProtocol::Oauth2)
        {
            return Err(Error::InvalidAccount("host does not require SSO"));
        }
        let mut pending = match vault.pending_yubi(&input.alias) {
            Ok(p) => {
                let (card, signing, pq) = if let Some(locator) = &p.locator {
                    (&locator.card, locator.signing_slot, locator.pq_slot)
                } else {
                    let preparation = p
                        .preparation
                        .as_ref()
                        .ok_or(Error::InvalidAccount("hardware preparation is missing"))?;
                    (
                        &preparation.card,
                        preparation.signing_slot,
                        preparation.pq_slot,
                    )
                };
                if card != &input.card || signing != input.signing_slot || pq != input.pq_slot {
                    return Err(Error::InvalidAccount(
                        "resume the original hardware card and slots",
                    ));
                }
                p
            }
            Err(Error::AccountMissing) => {
                if vault.contains(&input.alias)? {
                    return Err(Error::AccountExists);
                }
                let preparation = PendingYubiPreparation::new(
                    input.card,
                    input.signing_slot,
                    input.pq_slot,
                    input.retry_configuration,
                );
                let p = PendingYubiAccount::new_signup(
                    &input.alias,
                    &input.alias,
                    preparation,
                    input.device_name,
                    String::new(),
                    input.invite,
                    input.passphrase.as_ref().map(|p| p.expose().to_vec()),
                )?;
                vault.put_pending_yubi(&p)?;
                p
            }
            Err(e) => return Err(e),
        };
        if !matches!(pending.purpose, PendingYubiPurpose::Signup { .. }) {
            return Err(Error::InvalidAccount(
                "resume the original hardware operation",
            ));
        }
        let _prepared = vault.prepare_pending_yubi(&mut pending, &pin, provider)?;
        let intent = vault.pending_yubi_sso_intent(&input.alias)?;
        self.begin_bound_sso(&input.alias, intent, master, http)
    }
    #[allow(clippy::too_many_arguments)]
    pub fn finish_yubi_sso_signup(
        &self,
        alias: &str,
        id: [u8; 16],
        pin: Pin,
        provider: &dyn YubiProvider,
        vault: &mut AccountVault<'_>,
        master: &[u8; 32],
        http: &foks_oidc::ProviderHttp,
    ) -> Result<crate::SsoReport> {
        let flow = self.checked_sso_flow(alias, id, vault)?;
        self.profile.require(Capability::DeviceAdministration)?;
        if flow.purpose.is_existing() {
            return Err(Error::InvalidAccount("login flow cannot create an account"));
        }
        let host = self.pinned_host()?;
        let mut protected = self.mutation_store(master)?;
        if flow.final_operation.is_some() {
            self.resume_yubi_account(alias, pin, provider, vault, master)?;
        } else {
            let mut pending = vault.pending_yubi(alias)?;
            let prepared = vault.prepare_pending_yubi(&mut pending, &pin, provider)?;
            let auth = self.client.authorize_sso_signup(
                &host,
                id,
                foks_client::SsoSigningKey::Yubi(prepared.device.as_ref()),
                http,
                &mut protected,
            )?;
            pending.username = auth.username().into();
            let PendingYubiPurpose::Signup {
                device_name,
                email,
                invite,
                passphrase,
            } = &mut pending.purpose
            else {
                return Err(Error::InvalidAccount("hardware flow purpose differs"));
            };
            *email = auth.email().into();
            let request = YubiAccountRequest {
                username_utf8: auth.username().into(),
                device_name: device_name.clone(),
                email: email.clone(),
                invite_code: InviteCode::from_user_input(invite, true)?,
                passphrase: passphrase.as_ref().map(Passphrase::new).transpose()?,
                pq_hint: YubiSlotAndPqKeyId {
                    slot: u64::from(prepared.locator.pq_slot.get()),
                    id: prepared.locator.pq_key_id,
                },
            };
            vault.put_pending_yubi(&pending)?;
            let created = self.client.create_yubi_account_with_sso(
                &host,
                prepared.device.as_ref(),
                request,
                pending.signup_secrets()?,
                &self.paths.soft_database,
                &auth,
                &mut protected,
            )?;
            vault.commit_created_yubi(
                alias,
                &pending.username,
                &prepared.locator,
                &created.credential,
                None,
            )?;
            MutationCoordinator::new(&self.paths.hard_database, &mut protected)
                .finalize(&created.operation_id)?;
            vault.remove_pending_yubi(alias)?;
            drop(created);
            // Existing hardware completion owns management-key journaling and background jobs.
            self.resume_yubi_account(alias, pin, provider, vault, master)?;
        }
        let mut report = crate::sso::report(
            alias,
            foks_proto::SsoPurpose::Signup,
            self.client.sso_progress(&host, id, &mut protected)?,
        );
        // resume_yubi_account verified authenticated service access above.
        report.service_access = true;
        Ok(report)
    }
}

impl AccountVault<'_> {
    pub(crate) fn yubi_record_exportable(&mut self, alias: &str) -> Result<bool> {
        let stored = self.stored_yubi(alias)?;
        Ok(stored.pending_management_key.is_none()
            && (stored.management_key.is_none() || stored.management_enrolled))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use foks_keystore::{EncryptedFileSecretStore, MemorySecretStore, SecretStore as _};
    use foks_yubi::{MockYubiFailpoint, MockYubiProvider};

    fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
        haystack
            .windows(needle.len())
            .any(|candidate| candidate == needle)
    }

    fn pending_preparation(
        card: CardId,
        signing_slot: SlotId,
        pq_slot: SlotId,
    ) -> PendingYubiAccount {
        PendingYubiAccount::new_provision(
            "hardware-pending",
            "hardwareuser",
            PendingYubiPreparation::new(
                card,
                signing_slot,
                pq_slot,
                Some(PinRetryConfiguration::new(Pin::new("puk-42").unwrap(), 3, 3).unwrap()),
            ),
            "personal".to_owned(),
            "New security key".to_owned(),
            7,
        )
        .unwrap()
    }

    fn stored_management_state(
        current: [u8; 24],
        pending: Option<[u8; 24]>,
        enrolled: bool,
        generation: Option<u64>,
    ) -> StoredYubiAccount {
        let subkey_seed = [0x41; 32];
        let pq_public_key = [0x42; 33];
        StoredYubiAccount {
            version: YUBI_RECORD_VERSION,
            alias: "hardware".to_owned(),
            username: "hardwareuser".to_owned(),
            uid: [vec![ENTITY_USER], vec![0x43; 32]].concat(),
            locator: YubiDeviceLocator {
                card: CardId {
                    name: "test-card".to_owned(),
                    serial: 1,
                },
                signing_slot: SlotId::new(0x82).unwrap(),
                pq_slot: SlotId::new(0x83).unwrap(),
                signing_public_key: [0x44; 33],
                pq_public_key,
                pq_key_id: foks_crypto::yubi_pq_key_id(&pq_public_key).unwrap(),
            },
            subkey_id: derive_subkey_id(&SecretSeed::new(subkey_seed))
                .unwrap()
                .into_bytes(),
            subkey_seed,
            certificate_chain: vec![vec![0x45]],
            management_key: Some(current),
            pending_management_key: pending,
            management_enrolled: enrolled,
            management_generation: generation,
            management_refresh_source: None,
        }
    }

    #[test]
    fn interrupted_hardware_mutations_resume_from_encrypted_checkpoints() {
        let signing_slot = SlotId::new(0x82).unwrap();
        let pq_slot = SlotId::new(0x83).unwrap();
        let failpoints = [
            (MockYubiFailpoint::RetryReset, 0),
            (MockYubiFailpoint::PinRestore, 0),
            (MockYubiFailpoint::PukRestore, 0),
            (MockYubiFailpoint::KeyGeneration(signing_slot), 1),
            (MockYubiFailpoint::KeyGeneration(pq_slot), 2),
        ];

        for (index, (failpoint, expected_keys)) in failpoints.into_iter().enumerate() {
            let temporary = tempfile::tempdir().unwrap();
            let master_key = [u8::try_from(index + 1).unwrap(); 32];
            let provider = MockYubiProvider::with_card(
                "checkpoint-card",
                10_000 + u32::try_from(index).unwrap(),
                &Pin::new("pin-42").unwrap(),
            )
            .unwrap();
            let card = provider.cards().unwrap().remove(0);
            let mut pending = pending_preparation(card.clone(), signing_slot, pq_slot);
            let key = pending_yubi_key("hardware-pending");
            let record_path = temporary.path().join(format!("{key}.fks"));

            let mut store =
                EncryptedFileSecretStore::open(temporary.path(), Zeroizing::new(master_key))
                    .unwrap();
            {
                let mut vault = AccountVault::new(&mut store);
                vault.put_pending_yubi(&pending).unwrap();
                let encoded = std::fs::read(&record_path).unwrap();
                assert!(!contains_bytes(&encoded, b"pin-42"));
                assert!(!contains_bytes(&encoded, b"puk-42"));

                provider.fail_after_next(&card, failpoint).unwrap();
                assert!(
                    vault
                        .prepare_pending_yubi(&mut pending, &Pin::new("pin-42").unwrap(), &provider)
                        .is_err(),
                    "the post-mutation failpoint must interrupt preparation"
                );
                assert_eq!(provider.generated_key_count(&card).unwrap(), expected_keys);
                drop(pending);
            }
            drop(store);

            let encoded = std::fs::read(&record_path).unwrap();
            assert!(!contains_bytes(&encoded, b"pin-42"));
            assert!(!contains_bytes(&encoded, b"puk-42"));

            let mut store =
                EncryptedFileSecretStore::open(temporary.path(), Zeroizing::new(master_key))
                    .unwrap();
            {
                let mut vault = AccountVault::new(&mut store);
                let mut pending = vault.pending_yubi("hardware-pending").unwrap();
                let prepared = vault
                    .prepare_pending_yubi(&mut pending, &Pin::new("pin-42").unwrap(), &provider)
                    .unwrap();
                assert_eq!(provider.generated_key_count(&card).unwrap(), 2);
                assert_eq!(&prepared.locator, pending.locator().unwrap());
                assert_eq!(prepared.device.pin_retries().unwrap().remaining, 3);
                prepared
                    .device
                    .unblock_pin(&Pin::new("puk-42").unwrap(), &Pin::new("new-42").unwrap())
                    .expect("the intended PUK must be restored before key generation");
                provider
                    .open(&prepared.locator, Some(&Pin::new("new-42").unwrap()))
                    .expect("the resumed card must use the restored credential");
                drop(pending);
            }

            let completed = store.get(&key).unwrap();
            assert!(!contains_bytes(&completed, b"pin-42"));
            assert!(!contains_bytes(&completed, b"puk-42"));
        }
    }

    #[test]
    fn management_key_is_promoted_only_after_publication_completes() {
        let old = [0x11; 24];
        let next = [0x22; 24];
        let mut stored = stored_management_state(old, Some(next), false, None);

        assert_eq!(stored.management_key_for_publication().unwrap(), next);
        assert_eq!(stored.management_key, Some(old));
        assert_eq!(stored.pending_management_key, Some(next));
        assert!(!stored.management_enrolled);

        stored.complete_management_publication(7);
        assert_eq!(stored.management_key, Some(next));
        assert_eq!(stored.pending_management_key, None);
        assert!(stored.management_enrolled);
        assert_eq!(stored.management_generation, Some(7));
    }

    #[test]
    fn incomplete_rotation_blocks_recovery_and_refresh_paths() {
        let pending = stored_management_state([0x11; 24], Some([0x22; 24]), false, None);
        assert!(matches!(
            pending.require_completed_management_rotation(),
            Err(Error::InvalidAccount(
                "Yubi management-key rotation is incomplete; resume it first"
            ))
        ));

        let unpublished = stored_management_state([0x11; 24], None, false, None);
        assert!(unpublished.require_completed_management_rotation().is_err());

        let enrolled = stored_management_state([0x11; 24], None, true, Some(3));
        enrolled.require_completed_management_rotation().unwrap();
    }

    #[test]
    fn pending_rotation_cannot_be_marked_enrolled() {
        let stored = stored_management_state([0x11; 24], Some([0x22; 24]), true, Some(3));
        assert!(matches!(
            validate_stored_yubi(&stored, "hardware"),
            Err(Error::InvalidAccount(
                "pending Yubi management key is marked as enrolled"
            ))
        ));
    }

    #[test]
    fn yubi_account_states_are_returned_only_after_record_validation() {
        let stored = stored_management_state([0x11; 24], None, true, Some(3));
        let pending = PendingYubiAccount::new_provision(
            "hardware-pending",
            "hardwareuser",
            PendingYubiPreparation::new(
                stored.locator.card.clone(),
                stored.locator.signing_slot,
                stored.locator.pq_slot,
                None,
            ),
            "personal".to_owned(),
            "New security key".to_owned(),
            7,
        )
        .unwrap();
        let mut store = MemorySecretStore::default();
        {
            let mut vault = AccountVault::new(&mut store);
            vault.put_stored_yubi(&stored).unwrap();
            vault.put_pending_yubi(&pending).unwrap();
            assert_eq!(
                vault.yubi_accounts().unwrap(),
                vec![
                    YubiAccountSummary {
                        alias: "hardware".to_owned(),
                        state: YubiEnrollmentState::Complete,
                        device_id_hex: Some(hex(&[
                            &[foks_proto::ENTITY_YUBI][..],
                            &stored.locator.signing_public_key
                        ]
                        .concat())),
                        card_serial: Some(stored.locator.card.serial),
                    },
                    YubiAccountSummary {
                        alias: "hardware-pending".to_owned(),
                        state: YubiEnrollmentState::Pending,
                        device_id_hex: None,
                        card_serial: Some(stored.locator.card.serial),
                    },
                ]
            );
        }

        store
            .put(&pending_yubi_key("broken"), b"authenticated but malformed")
            .unwrap();
        assert!(AccountVault::new(&mut store).yubi_accounts().is_err());
    }
}
