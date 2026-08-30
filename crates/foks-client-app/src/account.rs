use super::*;

impl CheckedProfileSession<'_> {
    #[allow(clippy::too_many_arguments)]
    pub fn create_account(
        &self,
        alias: &str,
        username: &str,
        device_name: &str,
        email: &str,
        invite: &str,
        passphrase: Option<Passphrase>,
        vault: &mut AccountVault<'_>,
        master_key: &[u8; 32],
    ) -> Result<SyncReport> {
        self.profile.require(Capability::Signup)?;
        if passphrase.is_some() {
            self.profile.require(Capability::Passphrases)?;
        }
        validate_name(alias)?;
        if vault.contains(alias)? {
            return Err(Error::AccountExists);
        }
        let invite_code = InviteCode::from_user_input(invite, true)?;
        let pending = PendingSignup::random(alias, username)?;
        vault.put_pending(&pending)?;
        let host = self.pinned_host()?;
        let mut mutations = EncryptedFileMutationStore::open(
            &self.paths.protected_mutations,
            derive_mutation_key(master_key),
        )?;
        let created = self.client.create_software_account(
            &host,
            SoftwareAccountRequest {
                username_utf8: username.to_owned(),
                device_name: device_name.to_owned(),
                invite_code,
                email: email.to_owned(),
                passphrase,
            },
            pending.secrets()?,
            &self.paths.soft_database,
            &mut mutations,
        )?;
        vault.commit_created(alias, username, &created.credential)?;
        self.register_default_refresh_jobs_for(&created.credential.uid, now_microseconds()?)?;
        MutationCoordinator::new(&self.paths.hard_database, &mut mutations)
            .finalize(&created.operation_id)?;
        vault.remove_pending_signup(alias)?;
        Ok(SyncReport::from_created(&created))
    }

    pub fn resume_account(
        &self,
        alias: &str,
        vault: &mut AccountVault<'_>,
        master_key: &[u8; 32],
    ) -> Result<SyncReport> {
        self.profile.require(Capability::Signup)?;
        if vault.contains(alias)? {
            let loaded = vault.account(alias)?;
            let device = derive_device_public(&loaded.credential.seed)?;
            if let Some(operation) = HardStateStore::open(&self.paths.hard_database)?
                .latest_finalizable_mutation_for_binding(
                    self.pinned_host()?.host_id().as_bytes(),
                    MutationKind::Signup,
                    device.id.as_bytes(),
                    loaded.credential.uid.as_bytes(),
                )?
            {
                let mut mutations = EncryptedFileMutationStore::open(
                    &self.paths.protected_mutations,
                    derive_mutation_key(master_key),
                )?;
                MutationCoordinator::new(&self.paths.hard_database, &mut mutations)
                    .finalize(&operation.operation_id)?;
            }
            let _ = vault.store.remove(&pending_key(alias))?;
            return self.sync_account(alias, vault);
        }
        let pending = vault.pending(alias)?;
        let host = self.pinned_host()?;
        let mut uid =
            derive_shared_verify_key(&SecretSeed::new(pending.puk_seed), ENTITY_PUK_VERIFY)?
                .into_bytes();
        uid[0] = ENTITY_USER;
        let uid = EntityId::from_bytes(uid)?;
        let device = derive_device_public(&SecretSeed::new(pending.device_seed))?;
        let mut mutations = EncryptedFileMutationStore::open(
            &self.paths.protected_mutations,
            derive_mutation_key(master_key),
        )?;
        let created = self.client.resume_software_account_for_credential(
            &host,
            &uid,
            &device.id,
            &self.paths.soft_database,
            &mut mutations,
        )?;
        vault.commit_created(alias, &pending.username, &created.credential)?;
        self.register_default_refresh_jobs_for(&created.credential.uid, now_microseconds()?)?;
        MutationCoordinator::new(&self.paths.hard_database, &mut mutations)
            .finalize(&created.operation_id)?;
        vault.remove_pending_signup(alias)?;
        Ok(SyncReport::from_created(&created))
    }

    pub fn list_devices(
        &self,
        alias: &str,
        vault: &mut AccountVault<'_>,
    ) -> Result<Vec<DeviceSummary>> {
        self.profile.require(Capability::DeviceAdministration)?;
        let loaded = vault.account(alias)?;
        let host = self.pinned_host()?;
        let authenticated = self
            .client
            .authenticate_and_pin(&host, &loaded.credential)?;
        Ok(authenticated
            .verified
            .devices()
            .iter()
            .map(|device| DeviceSummary {
                id_hex: hex(device.id.as_bytes()),
                role: format!("{:?}", device.role.kind()).to_ascii_lowercase(),
                current: derive_device_public(&loaded.credential.seed)
                    .is_ok_and(|current| current.id == device.id),
            })
            .collect())
    }

    pub fn provision_owner_device(
        &self,
        source_alias: &str,
        target_alias: &str,
        device_name: &str,
        serial: u64,
        vault: &mut AccountVault<'_>,
        master_key: &[u8; 32],
    ) -> Result<DeviceProvisionReport> {
        self.profile.require(Capability::DeviceAdministration)?;
        validate_name(target_alias)?;
        if vault.contains(target_alias)? {
            return Err(Error::AccountExists);
        }
        let source = vault.account(source_alias)?;
        let pending = PendingDevice::random(source_alias, target_alias, &source.username, serial)?;
        vault.put_pending_device(&pending)?;
        let host = self.pinned_host()?;
        let mut mutations = EncryptedFileMutationStore::open(
            &self.paths.protected_mutations,
            derive_mutation_key(master_key),
        )?;
        let provisioned = self.client.provision_software_device(
            &host,
            &source.credential,
            SoftwareDeviceProvisionRequest {
                role: Role::OWNER,
                device_name: device_name.to_owned(),
                serial,
            },
            pending.secrets(),
            &mut mutations,
        )?;
        vault.commit_created(target_alias, &source.username, &provisioned.credential)?;
        self.register_default_refresh_jobs_for(&provisioned.credential.uid, now_microseconds()?)?;
        if let Some(operation_id) = provisioned.operation_id {
            MutationCoordinator::new(&self.paths.hard_database, &mut mutations)
                .finalize(&operation_id)?;
        }
        vault.remove_pending_device(target_alias)?;
        Ok(DeviceProvisionReport {
            alias: target_alias.to_owned(),
            device_id_hex: hex(derive_device_public(&provisioned.credential.seed)?
                .id
                .as_bytes()),
            user_chain_sequence: provisioned.authenticated.verified.chain_seqno(),
        })
    }

    pub fn resume_owner_device_provision(
        &self,
        target_alias: &str,
        vault: &mut AccountVault<'_>,
        master_key: &[u8; 32],
    ) -> Result<DeviceProvisionReport> {
        self.profile.require(Capability::DeviceAdministration)?;
        if vault.contains(target_alias)? {
            let target = vault.account(target_alias)?;
            let host = self.pinned_host()?;
            let device = derive_device_public(&target.credential.seed)?;
            if let Some(operation) = HardStateStore::open(&self.paths.hard_database)?
                .latest_finalizable_mutation_for_binding(
                    host.host_id().as_bytes(),
                    MutationKind::DeviceProvision,
                    target.credential.uid.as_bytes(),
                    device.id.as_bytes(),
                )?
            {
                let mut mutations = EncryptedFileMutationStore::open(
                    &self.paths.protected_mutations,
                    derive_mutation_key(master_key),
                )?;
                MutationCoordinator::new(&self.paths.hard_database, &mut mutations)
                    .finalize(&operation.operation_id)?;
            }
            let _ = vault.store.remove(&pending_device_key(target_alias))?;
            let authenticated = self
                .client
                .authenticate_and_pin(&host, &target.credential)?;
            self.register_default_refresh_jobs_for(&target.credential.uid, now_microseconds()?)?;
            return Ok(DeviceProvisionReport {
                alias: target_alias.to_owned(),
                device_id_hex: hex(device.id.as_bytes()),
                user_chain_sequence: authenticated.verified.chain_seqno(),
            });
        }
        let pending = vault.pending_device(target_alias)?;
        let source = vault.account(&pending.source_alias)?;
        let host = self.pinned_host()?;
        let device_seed = SecretSeed::new(pending.device_seed);
        let device = derive_device_public(&device_seed)?;
        let operation = HardStateStore::open(&self.paths.hard_database)?
            .pending_mutations(host.host_id().as_bytes())?
            .into_iter()
            .find(|operation| {
                operation.kind == MutationKind::DeviceProvision
                    && operation.scope_id == source.credential.uid.as_bytes()
                    && operation.subject_id == device.id.as_bytes()
            })
            .ok_or(Error::InvalidAccount(
                "pending device provision has no matching journal operation",
            ))?;
        let mut mutations = EncryptedFileMutationStore::open(
            &self.paths.protected_mutations,
            derive_mutation_key(master_key),
        )?;
        let provisioned = self.client.resume_software_device_provision(
            &host,
            &source.credential,
            operation.operation_id,
            device_seed,
            Role::OWNER,
            &mut mutations,
        )?;
        vault.commit_created(target_alias, &pending.username, &provisioned.credential)?;
        self.register_default_refresh_jobs_for(&provisioned.credential.uid, now_microseconds()?)?;
        if let Some(operation_id) = provisioned.operation_id {
            MutationCoordinator::new(&self.paths.hard_database, &mut mutations)
                .finalize(&operation_id)?;
        }
        vault.remove_pending_device(target_alias)?;
        Ok(DeviceProvisionReport {
            alias: target_alias.to_owned(),
            device_id_hex: hex(device.id.as_bytes()),
            user_chain_sequence: provisioned.authenticated.verified.chain_seqno(),
        })
    }

    pub fn start_owner_device_pairing(
        &self,
        account_alias: &str,
        vault: &mut AccountVault<'_>,
    ) -> Result<KexOfferReport> {
        self.profile.require(Capability::DeviceAdministration)?;
        if vault
            .store
            .keys()?
            .iter()
            .any(|key| key == &kex_offer_key(account_alias))
        {
            return Err(Error::AccountExists);
        }
        let account = vault.account(account_alias)?;
        let offer = foks_client::KexProvisionOffer::generate(Role::OWNER)?;
        let phrase = offer.phrase().expose_joined();
        let secret = *offer.into_secret_bytes();
        vault.put_kex_offer(&StoredKexOffer {
            version: CREDENTIAL_VERSION,
            account_alias: account_alias.to_owned(),
            secret,
        })?;
        let host = self.pinned_host()?;
        let restored = foks_client::KexProvisionOffer::from_secret_bytes(secret, Role::OWNER)?;
        self.client
            .publish_kex_provision_offer(&host, &account.credential, &restored)?;
        Ok(KexOfferReport {
            account_alias: account_alias.to_owned(),
            phrase: phrase.to_string(),
        })
    }

    pub fn republish_owner_device_pairing(
        &self,
        account_alias: &str,
        vault: &mut AccountVault<'_>,
    ) -> Result<KexOfferReport> {
        self.profile.require(Capability::DeviceAdministration)?;
        let account = vault.account(account_alias)?;
        let stored = vault.kex_offer(account_alias)?;
        let offer = foks_client::KexProvisionOffer::from_secret_bytes(stored.secret, Role::OWNER)?;
        let phrase = offer.phrase().expose_joined();
        let host = self.pinned_host()?;
        self.client
            .publish_kex_provision_offer(&host, &account.credential, &offer)?;
        Ok(KexOfferReport {
            account_alias: account_alias.to_owned(),
            phrase: phrase.to_string(),
        })
    }

    pub fn finish_owner_device_pairing(
        &self,
        account_alias: &str,
        vault: &mut AccountVault<'_>,
        master_key: &[u8; 32],
    ) -> Result<DeviceProvisionReport> {
        self.profile.require(Capability::DeviceAdministration)?;
        let account = vault.account(account_alias)?;
        let stored = vault.kex_offer(account_alias)?;
        let offer = foks_client::KexProvisionOffer::from_secret_bytes(stored.secret, Role::OWNER)?;
        let host = self.pinned_host()?;
        let mut mutations = EncryptedFileMutationStore::open(
            &self.paths.protected_mutations,
            derive_mutation_key(master_key),
        )?;
        let report = self.client.finish_kex_provisioning(
            &host,
            &account.credential,
            &offer,
            &mut mutations,
        )?;
        if let Some(operation_id) = report.operation_id {
            MutationCoordinator::new(&self.paths.hard_database, &mut mutations)
                .finalize(&operation_id)?;
        }
        let authenticated = self
            .client
            .authenticate_and_pin(&host, &account.credential)?;
        vault.remove_kex_offer(account_alias)?;
        Ok(DeviceProvisionReport {
            alias: account_alias.to_owned(),
            device_id_hex: hex(report.device.id.as_bytes()),
            user_chain_sequence: authenticated.verified.chain_seqno(),
        })
    }

    pub fn accept_owner_device_pairing(
        &self,
        input: KexAcceptanceInput,
        vault: &mut AccountVault<'_>,
    ) -> Result<DeviceProvisionReport> {
        self.profile.require(Capability::DeviceAdministration)?;
        validate_name(&input.target_alias)?;
        if vault.contains(&input.target_alias)? {
            return Err(Error::AccountExists);
        }
        foks_crypto::KexSecret::from_phrase(&input.phrase)?;
        let pending = PendingKexAcceptance {
            version: CREDENTIAL_VERSION,
            target_alias: input.target_alias.clone(),
            device_name: input.device_name.clone(),
            serial: input.serial,
            device_seed: random_array()?,
            phrase: input.phrase.clone(),
        };
        vault.put_pending_kex(&pending)?;
        self.finish_kex_acceptance(pending, vault)
    }

    pub fn resume_owner_device_pairing_acceptance(
        &self,
        target_alias: &str,
        vault: &mut AccountVault<'_>,
    ) -> Result<DeviceProvisionReport> {
        self.profile.require(Capability::DeviceAdministration)?;
        let pending = vault.pending_kex(target_alias)?;
        if vault.account_record_exists(target_alias)? {
            let account = vault.account(target_alias)?;
            let expected = derive_device_public(&SecretSeed::new(pending.device_seed))?;
            let actual = derive_device_public(&account.credential.seed)?;
            if expected.id != actual.id {
                return Err(Error::InvalidAccount(
                    "completed KEX credential binding changed",
                ));
            }
            let host = self.pinned_host()?;
            let authenticated = self
                .client
                .authenticate_and_pin(&host, &account.credential)?;
            self.register_default_refresh_jobs_for(&account.credential.uid, now_microseconds()?)?;
            vault.remove_pending_kex(target_alias)?;
            return Ok(DeviceProvisionReport {
                alias: target_alias.to_owned(),
                device_id_hex: hex(actual.id.as_bytes()),
                user_chain_sequence: authenticated.verified.chain_seqno(),
            });
        }
        self.finish_kex_acceptance(pending, vault)
    }

    fn finish_kex_acceptance(
        &self,
        pending: PendingKexAcceptance,
        vault: &mut AccountVault<'_>,
    ) -> Result<DeviceProvisionReport> {
        let host = self.pinned_host()?;
        let provisioned = self.client.accept_kex_provisioning(
            &host,
            &pending.phrase,
            &pending.device_name,
            pending.serial,
            SecretSeed::new(pending.device_seed),
        )?;
        let username = String::from_utf8_lossy(provisioned.authenticated.verified.username_utf8())
            .into_owned();
        vault.commit_created(&pending.target_alias, &username, &provisioned.credential)?;
        self.register_default_refresh_jobs_for(&provisioned.credential.uid, now_microseconds()?)?;
        vault.remove_pending_kex(&pending.target_alias)?;
        Ok(DeviceProvisionReport {
            alias: pending.target_alias.clone(),
            device_id_hex: hex(derive_device_public(&provisioned.credential.seed)?
                .id
                .as_bytes()),
            user_chain_sequence: provisioned.authenticated.verified.chain_seqno(),
        })
    }

    /// Generates and durably stores a backup key before submitting enrollment.
    /// The returned phrase should additionally be copied to offline storage.
    pub fn enroll_owner_backup(
        &self,
        account_alias: &str,
        backup_alias: &str,
        vault: &mut AccountVault<'_>,
    ) -> Result<Zeroizing<String>> {
        self.profile.require(Capability::Recovery)?;
        validate_name(backup_alias)?;
        if vault
            .store
            .keys()?
            .iter()
            .any(|key| key == &backup_key(backup_alias))
        {
            return Err(Error::AccountExists);
        }
        let loaded = vault.account(account_alias)?;
        let host = self.pinned_host()?;
        let backup = BackupKey::generate()?;
        let phrase = backup.phrase().expose_joined();
        vault.put_backup(backup_alias, account_alias, &phrase)?;
        self.client
            .enroll_backup_key(&host, &loaded.credential, Role::OWNER, &backup)?;
        Ok(phrase)
    }

    pub fn recover_owner_account(
        &self,
        target_alias: &str,
        phrase: Zeroizing<String>,
        device_name: &str,
        serial: u64,
        vault: &mut AccountVault<'_>,
    ) -> Result<DeviceProvisionReport> {
        self.profile.require(Capability::Recovery)?;
        validate_name(target_alias)?;
        if vault.contains(target_alias)? {
            return Err(Error::AccountExists);
        }
        let pending = PendingRecovery::random(target_alias, serial)?;
        vault.put_pending_recovery(&pending)?;
        self.finish_recovery(pending, phrase, device_name, vault)
    }

    pub fn resume_owner_recovery(
        &self,
        target_alias: &str,
        phrase: Zeroizing<String>,
        device_name: &str,
        vault: &mut AccountVault<'_>,
    ) -> Result<DeviceProvisionReport> {
        self.profile.require(Capability::Recovery)?;
        let pending = vault.pending_recovery(target_alias)?;
        self.finish_recovery(pending, phrase, device_name, vault)
    }

    fn finish_recovery(
        &self,
        pending: PendingRecovery,
        phrase: Zeroizing<String>,
        device_name: &str,
        vault: &mut AccountVault<'_>,
    ) -> Result<DeviceProvisionReport> {
        let host = self.pinned_host()?;
        let backup = BackupKey::from_phrase(&phrase)?;
        let located = self.client.load_backup_key(&host, backup)?;
        let recovered = self.client.recover_software_device(
            &host,
            located,
            SoftwareDeviceProvisionRequest {
                role: Role::OWNER,
                device_name: device_name.to_owned(),
                serial: pending.serial,
            },
            pending.secrets(),
        )?;
        let username =
            String::from_utf8_lossy(recovered.authenticated.verified.username()).into_owned();
        vault.commit_created(&pending.target_alias, &username, &recovered.credential)?;
        self.register_default_refresh_jobs_for(&recovered.credential.uid, now_microseconds()?)?;
        vault.remove_pending_recovery(&pending.target_alias)?;
        Ok(DeviceProvisionReport {
            alias: pending.target_alias.clone(),
            device_id_hex: hex(derive_device_public(&recovered.credential.seed)?
                .id
                .as_bytes()),
            user_chain_sequence: recovered.authenticated.verified.chain_seqno(),
        })
    }
    pub fn sync_account(&self, alias: &str, vault: &mut AccountVault<'_>) -> Result<SyncReport> {
        self.profile.require(Capability::UserSync)?;
        self.profile.require(Capability::Kv)?;
        let uid = vault.account(alias)?.credential.uid;
        self.register_default_refresh_jobs_for(&uid, now_microseconds()?)?;
        let (_, authenticated, directories) = self.authenticated_tree(alias, vault)?;
        Ok(SyncReport::from_tree(
            authenticated.verified.username(),
            authenticated.verified.chain_seqno(),
            &directories,
        ))
    }

    pub fn set_passphrase(
        &self,
        alias: &str,
        passphrase: Passphrase,
        vault: &mut AccountVault<'_>,
    ) -> Result<PassphraseReport> {
        self.profile.require(Capability::Passphrases)?;
        let loaded = vault.account(alias)?;
        let host = self.pinned_host()?;
        let metadata = self
            .client
            .set_passphrase(&host, &loaded.credential, &passphrase)?;
        let verification = self
            .client
            .verify_passphrase(&host, &loaded.credential, &passphrase)?;
        PassphraseReport::from_verified(metadata, verification)
    }

    pub fn change_passphrase(
        &self,
        alias: &str,
        passphrase: Passphrase,
        vault: &mut AccountVault<'_>,
    ) -> Result<PassphraseReport> {
        self.profile.require(Capability::Passphrases)?;
        let loaded = vault.account(alias)?;
        let host = self.pinned_host()?;
        let metadata = self
            .client
            .change_passphrase(&host, &loaded.credential, &passphrase)?;
        let verification = self
            .client
            .verify_passphrase(&host, &loaded.credential, &passphrase)?;
        PassphraseReport::from_verified(metadata, verification)
    }

    pub fn verify_passphrase(
        &self,
        alias: &str,
        passphrase: Passphrase,
        vault: &mut AccountVault<'_>,
    ) -> Result<PassphraseReport> {
        self.profile.require(Capability::Passphrases)?;
        let loaded = vault.account(alias)?;
        let host = self.pinned_host()?;
        let verification = self
            .client
            .verify_passphrase(&host, &loaded.credential, &passphrase)?;
        Ok(PassphraseReport {
            generation: verification.generation,
            stretch_version: "v1",
            verified: true,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PassphraseReport {
    pub generation: u64,
    pub stretch_version: &'static str,
    pub verified: bool,
}

impl PassphraseReport {
    pub(super) fn from_verified(
        metadata: foks_client::PassphraseMetadata,
        verification: foks_client::PassphraseVerification,
    ) -> Result<Self> {
        if metadata.generation != verification.generation {
            return Err(Error::InvalidAccount(
                "passphrase verification returned a different generation",
            ));
        }
        Ok(Self {
            generation: metadata.generation,
            stretch_version: match metadata.stretch_version {
                foks_proto::StretchVersion::V1 => "v1",
                foks_proto::StretchVersion::Test => "test",
            },
            verified: true,
        })
    }
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SyncReport {
    pub username: String,
    pub user_chain_sequence: u64,
    pub directories: usize,
    pub entries: usize,
}

impl SyncReport {
    fn from_created(created: &foks_client::CreatedSoftwareAccount) -> Self {
        Self::from_tree(
            created.authenticated.verified.username(),
            created.authenticated.verified.chain_seqno(),
            &created.kv_projection,
        )
    }

    pub(super) fn from_tree(
        username: &[u8],
        chain: u64,
        directories: &[KvDirectoryProjection],
    ) -> Self {
        Self {
            username: String::from_utf8_lossy(username).into_owned(),
            user_chain_sequence: chain,
            directories: directories.len(),
            entries: directories
                .iter()
                .map(|directory| directory.entries.len())
                .sum(),
        }
    }
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DeviceSummary {
    pub id_hex: String,
    pub role: String,
    pub current: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DeviceProvisionReport {
    pub alias: String,
    pub device_id_hex: String,
    pub user_chain_sequence: u64,
}
#[derive(Debug, Deserialize, Serialize)]
struct StoredAccount {
    version: u32,
    alias: String,
    username: String,
    uid: Vec<u8>,
    device_seed: [u8; 32],
    certificate_chain: Vec<Vec<u8>>,
}

impl Drop for StoredAccount {
    fn drop(&mut self) {
        self.device_seed.zeroize();
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub(super) struct PendingSignup {
    version: u32,
    alias: String,
    username: String,
    device_seed: [u8; 32],
    puk_seed: [u8; 32],
    self_token: [u8; 17],
}

impl Drop for PendingSignup {
    fn drop(&mut self) {
        self.device_seed.zeroize();
        self.puk_seed.zeroize();
        self.self_token.zeroize();
    }
}

#[derive(Debug, Deserialize, Serialize)]
struct PendingDevice {
    version: u32,
    source_alias: String,
    target_alias: String,
    username: String,
    serial: u64,
    device_seed: [u8; 32],
    self_token: [u8; 17],
}

#[derive(Deserialize, Serialize)]
struct StoredKexOffer {
    version: u32,
    account_alias: String,
    secret: [u8; foks_proto::KEX_SECRET_BYTES],
}

impl Drop for StoredKexOffer {
    fn drop(&mut self) {
        self.secret.zeroize();
    }
}

#[derive(Deserialize, Serialize)]
struct PendingKexAcceptance {
    version: u32,
    target_alias: String,
    device_name: String,
    serial: u64,
    device_seed: [u8; 32],
    phrase: String,
}

impl Drop for PendingKexAcceptance {
    fn drop(&mut self) {
        self.device_seed.zeroize();
        self.phrase.zeroize();
    }
}

impl PendingDevice {
    fn random(source_alias: &str, target_alias: &str, username: &str, serial: u64) -> Result<Self> {
        if serial == 0 {
            return Err(Error::InvalidAccount("device serial is zero"));
        }
        let mut device_seed = [0u8; 32];
        let mut self_token = [0u8; 17];
        getrandom::fill(&mut device_seed).map_err(|_| Error::Randomness)?;
        getrandom::fill(&mut self_token).map_err(|_| Error::Randomness)?;
        self_token[0] = 54;
        Ok(Self {
            version: CREDENTIAL_VERSION,
            source_alias: source_alias.to_owned(),
            target_alias: target_alias.to_owned(),
            username: username.to_owned(),
            serial,
            device_seed,
            self_token,
        })
    }

    fn secrets(&self) -> NewSoftwareDeviceSecrets {
        NewSoftwareDeviceSecrets::new(SecretSeed::new(self.device_seed), None, self.self_token)
    }
}

impl Drop for PendingDevice {
    fn drop(&mut self) {
        self.device_seed.zeroize();
        self.self_token.zeroize();
    }
}

#[derive(Debug, Deserialize, Serialize)]
struct PendingRecovery {
    version: u32,
    target_alias: String,
    serial: u64,
    device_seed: [u8; 32],
    self_token: [u8; 17],
}

impl PendingRecovery {
    fn random(target_alias: &str, serial: u64) -> Result<Self> {
        if serial == 0 {
            return Err(Error::InvalidAccount("recovery device serial is zero"));
        }
        let mut device_seed = [0u8; 32];
        let mut self_token = [0u8; 17];
        getrandom::fill(&mut device_seed).map_err(|_| Error::Randomness)?;
        getrandom::fill(&mut self_token).map_err(|_| Error::Randomness)?;
        self_token[0] = 54;
        Ok(Self {
            version: CREDENTIAL_VERSION,
            target_alias: target_alias.to_owned(),
            serial,
            device_seed,
            self_token,
        })
    }

    fn secrets(&self) -> NewSoftwareDeviceSecrets {
        NewSoftwareDeviceSecrets::new(SecretSeed::new(self.device_seed), None, self.self_token)
    }
}

impl Drop for PendingRecovery {
    fn drop(&mut self) {
        self.device_seed.zeroize();
        self.self_token.zeroize();
    }
}

#[derive(Debug, Deserialize, Serialize)]
struct StoredBackup {
    version: u32,
    backup_alias: String,
    account_alias: String,
    phrase: String,
}

impl Drop for StoredBackup {
    fn drop(&mut self) {
        self.phrase.zeroize();
    }
}
impl PendingSignup {
    pub(super) fn random(alias: &str, username: &str) -> Result<Self> {
        let mut device_seed = [0u8; 32];
        let mut puk_seed = [0u8; 32];
        let mut self_token = [0u8; 17];
        getrandom::fill(&mut device_seed).map_err(|_| Error::Randomness)?;
        getrandom::fill(&mut puk_seed).map_err(|_| Error::Randomness)?;
        getrandom::fill(&mut self_token).map_err(|_| Error::Randomness)?;
        self_token[0] = 54;
        Ok(Self {
            version: CREDENTIAL_VERSION,
            alias: alias.to_owned(),
            username: username.to_owned(),
            device_seed,
            puk_seed,
            self_token,
        })
    }

    fn secrets(&self) -> Result<SoftwareAccountSecrets> {
        validate_pending(self)?;
        Ok(SoftwareAccountSecrets::new(
            SecretSeed::new(self.device_seed),
            SecretSeed::new(self.puk_seed),
            self.self_token,
        ))
    }
}

pub struct LoadedAccount {
    pub alias: String,
    pub username: String,
    pub credential: DeviceCredential,
}

#[derive(Clone, Eq, PartialEq, Serialize)]
pub struct KexOfferReport {
    pub account_alias: String,
    pub phrase: String,
}

impl std::fmt::Debug for KexOfferReport {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("KexOfferReport")
            .field("account_alias", &self.account_alias)
            .field("phrase", &"[REDACTED]")
            .finish()
    }
}

impl Drop for KexOfferReport {
    fn drop(&mut self) {
        self.phrase.zeroize();
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct KexAcceptanceInput {
    pub target_alias: String,
    pub device_name: String,
    pub serial: u64,
    pub phrase: String,
}

impl std::fmt::Debug for KexAcceptanceInput {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("KexAcceptanceInput")
            .field("target_alias", &self.target_alias)
            .field("device_name", &self.device_name)
            .field("serial", &self.serial)
            .field("phrase", &"[REDACTED]")
            .finish()
    }
}

impl Drop for KexAcceptanceInput {
    fn drop(&mut self) {
        self.phrase.zeroize();
    }
}

pub struct AccountVault<'a> {
    pub(super) store: &'a mut dyn SecretStore,
}

impl<'a> AccountVault<'a> {
    pub fn new(store: &'a mut dyn SecretStore) -> Self {
        Self { store }
    }

    pub fn aliases(&mut self) -> Result<Vec<String>> {
        Ok(self
            .store
            .keys()?
            .into_iter()
            .filter_map(|key| key.strip_prefix("account.").map(str::to_owned))
            .collect())
    }

    pub fn contains(&mut self, alias: &str) -> Result<bool> {
        validate_name(alias)?;
        Ok(self.store.keys()?.iter().any(|key| {
            key == &account_key(alias)
                || key == &pending_key(alias)
                || key == &pending_device_key(alias)
                || key == &pending_recovery_key(alias)
                || key == &pending_kex_key(alias)
                || key == &super::yubi::yubi_account_key(alias)
                || key == &super::yubi::pending_yubi_key(alias)
        }))
    }

    fn account_record_exists(&mut self, alias: &str) -> Result<bool> {
        validate_name(alias)?;
        Ok(self
            .store
            .keys()?
            .iter()
            .any(|key| key == &account_key(alias)))
    }

    fn put_kex_offer(&mut self, offer: &StoredKexOffer) -> Result<()> {
        validate_name(&offer.account_alias)?;
        if offer.version != CREDENTIAL_VERSION {
            return Err(Error::InvalidAccount("KEX offer version is invalid"));
        }
        let encoded = Zeroizing::new(serde_json::to_vec(offer)?);
        self.store
            .put(&kex_offer_key(&offer.account_alias), &encoded)?;
        Ok(())
    }

    fn kex_offer(&mut self, account_alias: &str) -> Result<StoredKexOffer> {
        validate_name(account_alias)?;
        let bytes = self
            .store
            .get(&kex_offer_key(account_alias))
            .map_err(|error| match error {
                foks_keystore::Error::Missing => Error::AccountMissing,
                other => Error::Keystore(other),
            })?;
        let offer: StoredKexOffer = serde_json::from_slice(&bytes)?;
        if offer.version != CREDENTIAL_VERSION || offer.account_alias != account_alias {
            return Err(Error::InvalidAccount("KEX offer binding changed"));
        }
        Ok(offer)
    }

    fn remove_kex_offer(&mut self, account_alias: &str) -> Result<()> {
        self.store.remove(&kex_offer_key(account_alias))?;
        Ok(())
    }

    fn put_pending_kex(&mut self, pending: &PendingKexAcceptance) -> Result<()> {
        validate_pending_kex(pending)?;
        let encoded = Zeroizing::new(serde_json::to_vec(pending)?);
        self.store
            .put(&pending_kex_key(&pending.target_alias), &encoded)?;
        Ok(())
    }

    fn pending_kex(&mut self, target_alias: &str) -> Result<PendingKexAcceptance> {
        validate_name(target_alias)?;
        let bytes =
            self.store
                .get(&pending_kex_key(target_alias))
                .map_err(|error| match error {
                    foks_keystore::Error::Missing => Error::AccountMissing,
                    other => Error::Keystore(other),
                })?;
        let pending: PendingKexAcceptance = serde_json::from_slice(&bytes)?;
        validate_pending_kex(&pending)?;
        if pending.target_alias != target_alias {
            return Err(Error::InvalidAccount("pending KEX alias binding changed"));
        }
        Ok(pending)
    }

    fn remove_pending_kex(&mut self, target_alias: &str) -> Result<()> {
        self.store.remove(&pending_kex_key(target_alias))?;
        Ok(())
    }
    pub fn account(&mut self, alias: &str) -> Result<LoadedAccount> {
        validate_name(alias)?;
        let bytes = self
            .store
            .get(&account_key(alias))
            .map_err(|error| match error {
                foks_keystore::Error::Missing => Error::AccountMissing,
                other => Error::Keystore(other),
            })?;
        let stored: StoredAccount = serde_json::from_slice(&bytes)?;
        validate_account(&stored, alias)?;
        Ok(LoadedAccount {
            alias: stored.alias.clone(),
            username: stored.username.clone(),
            credential: DeviceCredential {
                uid: EntityId::from_bytes(stored.uid.clone())?,
                seed: SecretSeed::new(stored.device_seed),
                certificate_chain: stored.certificate_chain.clone(),
            },
        })
    }

    pub(super) fn pending(&mut self, alias: &str) -> Result<PendingSignup> {
        validate_name(alias)?;
        let bytes = self
            .store
            .get(&pending_key(alias))
            .map_err(|error| match error {
                foks_keystore::Error::Missing => Error::AccountMissing,
                other => Error::Keystore(other),
            })?;
        let pending: PendingSignup = serde_json::from_slice(&bytes)?;
        validate_pending(&pending)?;
        if pending.alias != alias {
            return Err(Error::InvalidAccount("pending alias binding changed"));
        }
        Ok(pending)
    }

    pub(super) fn put_pending(&mut self, pending: &PendingSignup) -> Result<()> {
        validate_pending(pending)?;
        let encoded = Zeroizing::new(serde_json::to_vec(pending)?);
        self.store.put(&pending_key(&pending.alias), &encoded)?;
        Ok(())
    }

    pub(super) fn remove_pending_signup(&mut self, alias: &str) -> Result<()> {
        self.store.remove(&pending_key(alias))?;
        Ok(())
    }

    fn pending_device(&mut self, target_alias: &str) -> Result<PendingDevice> {
        validate_name(target_alias)?;
        let bytes =
            self.store
                .get(&pending_device_key(target_alias))
                .map_err(|error| match error {
                    foks_keystore::Error::Missing => Error::AccountMissing,
                    other => Error::Keystore(other),
                })?;
        let pending: PendingDevice = serde_json::from_slice(&bytes)?;
        validate_pending_device(&pending)?;
        if pending.target_alias != target_alias {
            return Err(Error::InvalidAccount(
                "pending device alias binding changed",
            ));
        }
        Ok(pending)
    }

    fn put_pending_device(&mut self, pending: &PendingDevice) -> Result<()> {
        validate_pending_device(pending)?;
        let encoded = Zeroizing::new(serde_json::to_vec(pending)?);
        self.store
            .put(&pending_device_key(&pending.target_alias), &encoded)?;
        Ok(())
    }

    fn remove_pending_device(&mut self, target_alias: &str) -> Result<()> {
        self.store.remove(&pending_device_key(target_alias))?;
        Ok(())
    }

    fn pending_recovery(&mut self, target_alias: &str) -> Result<PendingRecovery> {
        validate_name(target_alias)?;
        let bytes = self
            .store
            .get(&pending_recovery_key(target_alias))
            .map_err(|error| match error {
                foks_keystore::Error::Missing => Error::AccountMissing,
                other => Error::Keystore(other),
            })?;
        let pending: PendingRecovery = serde_json::from_slice(&bytes)?;
        validate_pending_recovery(&pending)?;
        if pending.target_alias != target_alias {
            return Err(Error::InvalidAccount(
                "pending recovery alias binding changed",
            ));
        }
        Ok(pending)
    }

    fn put_pending_recovery(&mut self, pending: &PendingRecovery) -> Result<()> {
        validate_pending_recovery(pending)?;
        let encoded = Zeroizing::new(serde_json::to_vec(pending)?);
        self.store
            .put(&pending_recovery_key(&pending.target_alias), &encoded)?;
        Ok(())
    }

    fn remove_pending_recovery(&mut self, target_alias: &str) -> Result<()> {
        self.store.remove(&pending_recovery_key(target_alias))?;
        Ok(())
    }

    fn put_backup(&mut self, backup_alias: &str, account_alias: &str, phrase: &str) -> Result<()> {
        validate_name(backup_alias)?;
        validate_name(account_alias)?;
        if phrase.len() > 1024 || phrase.split_whitespace().count() != 17 {
            return Err(Error::InvalidAccount("backup phrase is malformed"));
        }
        let backup = StoredBackup {
            version: CREDENTIAL_VERSION,
            backup_alias: backup_alias.to_owned(),
            account_alias: account_alias.to_owned(),
            phrase: phrase.to_owned(),
        };
        let encoded = Zeroizing::new(serde_json::to_vec(&backup)?);
        self.store.put(&backup_key(backup_alias), &encoded)?;
        Ok(())
    }

    pub(super) fn commit_created(
        &mut self,
        alias: &str,
        username: &str,
        credential: &DeviceCredential,
    ) -> Result<()> {
        let stored = StoredAccount {
            version: CREDENTIAL_VERSION,
            alias: alias.to_owned(),
            username: username.to_owned(),
            uid: credential.uid.as_bytes().to_vec(),
            device_seed: *credential.seed.as_bytes(),
            certificate_chain: credential.certificate_chain.clone(),
        };
        validate_account(&stored, alias)?;
        let encoded = Zeroizing::new(serde_json::to_vec(&stored)?);
        self.store.put(&account_key(alias), &encoded)?;
        Ok(())
    }
}

pub fn derive_vault_key(master_key: &[u8; 32]) -> Zeroizing<[u8; 32]> {
    Zeroizing::new(prefixed_hash(VAULT_KEY_TYPE_ID, master_key))
}

pub fn derive_mutation_key(master_key: &[u8; 32]) -> Zeroizing<[u8; 32]> {
    Zeroizing::new(prefixed_hash(MUTATION_KEY_TYPE_ID, master_key))
}

fn validate_account(account: &StoredAccount, expected_alias: &str) -> Result<()> {
    if account.version != CREDENTIAL_VERSION || account.alias != expected_alias {
        return Err(Error::InvalidAccount("version or alias binding changed"));
    }
    validate_name(&account.alias)?;
    if account.username.is_empty() || account.username.len() > 256 {
        return Err(Error::InvalidAccount("username is missing or excessive"));
    }
    EntityId::from_bytes(account.uid.clone())?.require_type(ENTITY_USER)?;
    validate_certificates(&account.certificate_chain)
}

fn validate_pending(pending: &PendingSignup) -> Result<()> {
    if pending.version != CREDENTIAL_VERSION || pending.self_token[0] != 54 {
        return Err(Error::InvalidAccount(
            "pending signup version or token is invalid",
        ));
    }
    validate_name(&pending.alias)?;
    if pending.username.is_empty() || pending.username.len() > 256 {
        return Err(Error::InvalidAccount(
            "pending username is missing or excessive",
        ));
    }
    Ok(())
}

fn validate_pending_device(pending: &PendingDevice) -> Result<()> {
    if pending.version != CREDENTIAL_VERSION || pending.self_token[0] != 54 || pending.serial == 0 {
        return Err(Error::InvalidAccount(
            "pending device version, token, or serial is invalid",
        ));
    }
    validate_name(&pending.source_alias)?;
    validate_name(&pending.target_alias)?;
    if pending.username.is_empty() || pending.username.len() > 256 {
        return Err(Error::InvalidAccount(
            "pending device username is missing or excessive",
        ));
    }
    Ok(())
}

fn validate_pending_kex(pending: &PendingKexAcceptance) -> Result<()> {
    if pending.version != CREDENTIAL_VERSION
        || pending.serial == 0
        || pending.device_name.is_empty()
        || pending.device_name.len() > 256
    {
        return Err(Error::InvalidAccount(
            "pending KEX version, device name, or serial is invalid",
        ));
    }
    validate_name(&pending.target_alias)?;
    foks_crypto::KexSecret::from_phrase(&pending.phrase)?;
    Ok(())
}

fn validate_pending_recovery(pending: &PendingRecovery) -> Result<()> {
    if pending.version != CREDENTIAL_VERSION || pending.self_token[0] != 54 || pending.serial == 0 {
        return Err(Error::InvalidAccount(
            "pending recovery version, token, or serial is invalid",
        ));
    }
    validate_name(&pending.target_alias)
}

pub(super) fn validate_certificates(certificates: &[Vec<u8>]) -> Result<()> {
    if certificates.is_empty()
        || certificates.len() > MAX_CERTIFICATES
        || certificates
            .iter()
            .any(|certificate| certificate.is_empty() || certificate.len() > MAX_CERTIFICATE_BYTES)
    {
        return Err(Error::InvalidAccount(
            "certificate chain is invalid or excessive",
        ));
    }
    Ok(())
}
