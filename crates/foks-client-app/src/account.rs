use super::*;

#[cfg(test)]
static TEST_FAIL_AFTER_BACKUP_ENROLLMENT: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);
#[cfg(test)]
static TEST_FAIL_AFTER_BACKUP_REVOCATION: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

impl CheckedProfileSession<'_> {
    pub(super) fn with_account_credential<T>(
        &self,
        alias: &str,
        parent: Option<&dyn foks_crypto::YubiDevice>,
        vault: &mut AccountVault<'_>,
        f: impl FnOnce(foks_client::FederationCredential<'_, '_>) -> Result<T>,
    ) -> Result<T> {
        match vault.account(alias) {
            Ok(account) => f(foks_client::FederationCredential::Software(
                &account.credential,
            )),
            Err(Error::AccountMissing) => {
                let account = vault.yubi_account(alias)?;
                let parent = parent.ok_or_else(|| {
                    Error::YubiUnlockRequired("unlock the selected account key".into())
                })?;
                let credential = account.credential(parent);
                if parent.entity_id().p256_key()? != account.locator.signing_public_key {
                    return Err(Error::InvalidAccount("wrong account hardware key"));
                }
                f(foks_client::FederationCredential::Yubi(&credential))
            }
            Err(error) => Err(error),
        }
    }

    pub(super) fn mutation_store(&self, master: &[u8; 32]) -> Result<EncryptedFileMutationStore> {
        Ok(EncryptedFileMutationStore::open(
            &self.paths.protected_mutations,
            derive_mutation_key(master),
        )?)
    }

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
        let host = self.pinned_host()?;
        let pending = PendingSignup::random(alias, username)?;
        let mut mutations = EncryptedFileMutationStore::open(
            &self.paths.protected_mutations,
            derive_mutation_key(master_key),
        )?;
        vault.put_pending(&pending)?;
        let result = self.client.create_software_account(
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
        );
        let created = match result {
            Ok(created) => created,
            Err(error) => {
                // Only discard an alias when the durable journal proves that
                // signup never reached submission. An ambiguous server result
                // must retain its original keys and exact retry material.
                if pending
                    .journal_operation(&host, &self.paths.hard_database)?
                    .is_none()
                {
                    vault.remove_pending_signup(alias)?;
                }
                return Err(error.into());
            }
        };
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
        if vault.account_record_exists(alias)? {
            let loaded = vault.account(alias)?;
            let device = loaded.credential.public_material()?;
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
        let Some(operation) = pending.journal_operation(&host, &self.paths.hard_database)? else {
            let mut uid =
                derive_shared_verify_key(&SecretSeed::new(pending.puk_seed), ENTITY_PUK_VERIFY)?
                    .into_bytes();
            uid[0] = ENTITY_USER;
            if HardStateStore::open(&self.paths.hard_database)?
                .sso_flows(host.host_id().as_bytes(), &uid)?
                .iter()
                .any(|flow| {
                    matches!(
                        flow.state,
                        foks_client_db::SsoFlowState::Prepared
                            | foks_client_db::SsoFlowState::AwaitingBrowser
                            | foks_client_db::SsoFlowState::Ready
                            | foks_client_db::SsoFlowState::Binding
                            | foks_client_db::SsoFlowState::Unknown
                    )
                })
            {
                return Err(Error::InvalidAccount(
                    "resume the account's organization sign-in flow",
                ));
            }
            // A crash during preflight can leave only the protected seeds.
            // No journal means signup could not have been sent to the server.
            vault.remove_pending_signup(alias)?;
            return Err(Error::InvalidAccount(
                "signup stopped before submission; create the account again with this alias",
            ));
        };
        let mut mutations = EncryptedFileMutationStore::open(
            &self.paths.protected_mutations,
            derive_mutation_key(master_key),
        )?;
        let created = self.client.resume_software_account(
            &host,
            operation.operation_id,
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
                name: authenticated
                    .verified
                    .device_display_name(&device.id)
                    .map(str::to_owned),
                role: format!("{:?}", device.role.kind()).to_ascii_lowercase(),
                current: loaded
                    .credential
                    .public_material()
                    .is_ok_and(|current| current.id == device.id),
            })
            .collect())
    }

    pub fn remove_software_device(
        &self,
        signer_alias: &str,
        device_id_hex: &str,
        vault: &mut AccountVault<'_>,
        master_key: &[u8; 32],
    ) -> Result<DeviceRevocationReport> {
        self.profile.require(Capability::DeviceAdministration)?;
        let signer = vault.account(signer_alias)?;
        let target = entity_id_from_hex(device_id_hex)?;
        target.clone().require_type(foks_proto::ENTITY_DEVICE)?;
        let host = self.pinned_host()?;
        let authenticated = self
            .client
            .authenticate_and_pin(&host, &signer.credential)?;
        let Some(target_role) = authenticated
            .verified
            .devices()
            .iter()
            .find(|device| device.id == target)
            .map(|device| device.role)
        else {
            return Ok(DeviceRevocationReport {
                device_id_hex: hex(target.as_bytes()),
                user_chain_sequence: authenticated.verified.chain_seqno(),
                already_absent: true,
            });
        };
        let (rotations, no_passphrase) = self.software_revocation_material(
            &host,
            &signer.credential,
            &authenticated,
            target_role,
        )?;
        let mut mutations = EncryptedFileMutationStore::open(
            &self.paths.protected_mutations,
            derive_mutation_key(master_key),
        )?;
        let revoked = self.client.revoke_user_credential_with_software_device(
            &host,
            &signer.credential,
            &target,
            &rotations,
            no_passphrase,
            &mut mutations,
        )?;
        Ok(DeviceRevocationReport {
            device_id_hex: hex(target.as_bytes()),
            user_chain_sequence: revoked.verified.chain_seqno(),
            already_absent: false,
        })
    }

    pub(super) fn software_revocation_material(
        &self,
        host: &foks_client::PinnedHost,
        signer: &DeviceCredential,
        authenticated: &AuthenticatedUserOutcome,
        target_role: Role,
    ) -> Result<(
        Vec<foks_client::UserPukRotation>,
        Option<foks_client::NoPassphraseConfigured>,
    )> {
        let mut rotations = Vec::new();
        for public in authenticated
            .verified
            .shared_keys()
            .iter()
            .filter(|key| key.role <= target_role)
        {
            let previous = self
                .client
                .load_puks_for_role(host, signer, &authenticated.verified, public.role)?
                .into_iter()
                .find(|puk| puk.role == public.role && puk.generation == public.generation)
                .ok_or(Error::InvalidAccount(
                    "current PUK required for credential revocation is unavailable",
                ))?;
            rotations.push(foks_client::UserPukRotation {
                role: public.role,
                previous_generation: public.generation,
                previous_seed: previous.seed,
                new_seed: SecretSeed::new(random_array()?),
            });
        }
        let no_passphrase = if rotations
            .iter()
            .any(|rotation| rotation.role == Role::OWNER)
        {
            self.profile.require(Capability::Passphrases)?;
            match self
                .client
                .authenticated_passphrase_settings(host, signer, authenticated)?
            {
                Some(_) => None,
                None => {
                    if !HardStateStore::open(&self.paths.hard_database)?
                        .user_has_no_passphrase_attestation(
                            host.host_id().as_bytes(),
                            signer.uid.as_bytes(),
                        )?
                    {
                        return Err(Error::InvalidAccount(
                            "legacy unlinked passphrase state must be verified before owner rotation",
                        ));
                    }
                    Some(foks_client::NoPassphraseConfigured)
                }
            }
        } else {
            None
        };
        Ok((rotations, no_passphrase))
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
        let host = self.pinned_host()?;
        let pending = PendingDevice::random(source_alias, target_alias, &source.username, serial)?;
        let mut mutations = EncryptedFileMutationStore::open(
            &self.paths.protected_mutations,
            derive_mutation_key(master_key),
        )?;
        vault.put_pending_device(&pending)?;
        let result = self.client.provision_software_device(
            &host,
            &source.credential,
            SoftwareDeviceProvisionRequest {
                role: Role::OWNER,
                device_name: device_name.to_owned(),
                serial,
            },
            pending.secrets(),
            &mut mutations,
        );
        let provisioned = match result {
            Ok(provisioned) => provisioned,
            Err(error) => {
                let device = derive_device_public(&SecretSeed::new(pending.device_seed))?;
                if HardStateStore::open(&self.paths.hard_database)?
                    .latest_mutation_for_binding(
                        host.host_id().as_bytes(),
                        MutationKind::DeviceProvision,
                        source.credential.uid.as_bytes(),
                        device.id.as_bytes(),
                    )?
                    .is_none()
                {
                    vault.remove_pending_device(target_alias)?;
                }
                return Err(error.into());
            }
        };
        vault.commit_created(target_alias, &source.username, &provisioned.credential)?;
        self.register_default_refresh_jobs_for(&provisioned.credential.uid, now_microseconds()?)?;
        if let Some(operation_id) = provisioned.operation_id {
            MutationCoordinator::new(&self.paths.hard_database, &mut mutations)
                .finalize(&operation_id)?;
        }
        vault.remove_pending_device(target_alias)?;
        Ok(DeviceProvisionReport {
            alias: target_alias.to_owned(),
            device_id_hex: hex(provisioned.credential.public_material()?.id.as_bytes()),
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
        if vault.account_record_exists(target_alias)? {
            let target = vault.account(target_alias)?;
            let host = self.pinned_host()?;
            let device = target.credential.public_material()?;
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
        let Some(operation) = HardStateStore::open(&self.paths.hard_database)?
            .latest_mutation_for_binding(
                host.host_id().as_bytes(),
                MutationKind::DeviceProvision,
                source.credential.uid.as_bytes(),
                device.id.as_bytes(),
            )?
        else {
            vault.remove_pending_device(target_alias)?;
            return Err(Error::InvalidAccount(
                "device provisioning stopped before submission; provision again with this alias",
            ));
        };
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

    pub fn import_software_device(
        &self,
        target_alias: &str,
        expected_user: &[u8; 33],
        expected_device: &[u8; 33],
        device_seed: SecretSeed,
        vault: &mut AccountVault<'_>,
    ) -> Result<DeviceProvisionReport> {
        self.profile.require(Capability::UserSync)?;
        validate_name(target_alias)?;
        for alias in vault.aliases()? {
            let existing = vault.account(&alias)?;
            let device = existing.credential.public_material()?;
            if existing.credential.uid.as_bytes() == expected_user
                && device.id.as_bytes() == expected_device
            {
                if alias != target_alias {
                    return Err(Error::AccountExists);
                }
                let host = self.pinned_host()?;
                let authenticated = self
                    .client
                    .authenticate_and_pin(&host, &existing.credential)?;
                self.register_default_refresh_jobs_for(
                    &existing.credential.uid,
                    now_microseconds()?,
                )?;
                return Ok(DeviceProvisionReport {
                    alias: target_alias.to_owned(),
                    device_id_hex: hex(device.id.as_bytes()),
                    user_chain_sequence: authenticated.verified.chain_seqno(),
                });
            }
        }
        if vault.contains(target_alias)? {
            return Err(Error::AccountExists);
        }
        let uid = EntityId::from_bytes(expected_user.to_vec())?;
        let device = derive_device_public(&device_seed)?;
        if device.id.as_bytes() != expected_device {
            return Err(Error::InvalidAccount(
                "imported device seed does not match the selected profile",
            ));
        }
        let host = self.pinned_host()?;
        let certificate_chain =
            self.client
                .fetch_device_certificate_chain(&host, &uid, &device_seed)?;
        let credential = DeviceCredential {
            key_kind: foks_client::SoftwareKeyKind::Device,
            uid,
            seed: device_seed,
            certificate_chain,
        };
        let authenticated = self.client.authenticate_and_pin(&host, &credential)?;
        if authenticated.verified.uid().as_bytes() != expected_user
            || !authenticated
                .verified
                .devices()
                .iter()
                .any(|entry| entry.id.as_bytes() == expected_device)
        {
            return Err(Error::InvalidAccount(
                "imported device is not enrolled for the selected account",
            ));
        }
        let username = String::from_utf8_lossy(authenticated.verified.username_utf8()).into_owned();
        vault.commit_created(target_alias, &username, &credential)?;
        self.register_default_refresh_jobs_for(&credential.uid, now_microseconds()?)?;
        Ok(DeviceProvisionReport {
            alias: target_alias.to_owned(),
            device_id_hex: hex(device.id.as_bytes()),
            user_chain_sequence: authenticated.verified.chain_seqno(),
        })
    }

    pub fn start_owner_device_pairing(
        &self,
        account_alias: &str,
        vault: &mut AccountVault<'_>,
    ) -> Result<KexOfferReport> {
        self.profile.require(Capability::DeviceAdministration)?;
        let _ = vault.remove_kex_offer(account_alias);
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
        let report = self.client.finish_kex_provisioning_within(
            &host,
            &account.credential,
            &offer,
            &mut mutations,
            self.pairing_budget(),
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
        self.accept_owner_device_pairing_for_user(input, None, None, vault)
    }

    pub fn accept_owner_device_pairing_for_user(
        &self,
        input: KexAcceptanceInput,
        expected_user: Option<&[u8; 33]>,
        source_candidate_id: Option<&str>,
        vault: &mut AccountVault<'_>,
    ) -> Result<DeviceProvisionReport> {
        self.profile.require(Capability::DeviceAdministration)?;
        validate_name(&input.target_alias)?;
        if vault.account_record_exists(&input.target_alias)? {
            return Err(Error::AccountExists);
        }
        let _ = vault.remove_pending_kex(&input.target_alias);
        foks_crypto::KexSecret::from_phrase(&input.phrase)?;
        if source_candidate_id.is_some_and(|id| {
            id.len() != 64
                || !id
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        }) {
            return Err(Error::InvalidAccount("source candidate id is invalid"));
        }
        let pending = PendingKexAcceptance {
            version: CREDENTIAL_VERSION,
            target_alias: input.target_alias.clone(),
            device_name: input.device_name.clone(),
            serial: input.serial,
            device_seed: random_array()?,
            phrase: input.phrase.clone(),
            source_candidate_id: source_candidate_id.map(str::to_owned),
            expected_user: expected_user.map(|user| user.to_vec()),
        };
        vault.put_pending_kex(&pending)?;
        self.finish_kex_acceptance(pending, expected_user, vault)
    }

    pub fn resume_owner_device_pairing_acceptance(
        &self,
        target_alias: &str,
        vault: &mut AccountVault<'_>,
    ) -> Result<DeviceProvisionReport> {
        self.resume_owner_device_pairing_acceptance_for_user(target_alias, None, None, vault)
    }

    pub fn resume_owner_device_pairing_acceptance_for_user(
        &self,
        target_alias: &str,
        expected_user: Option<&[u8; 33]>,
        source_candidate_id: Option<&str>,
        vault: &mut AccountVault<'_>,
    ) -> Result<DeviceProvisionReport> {
        self.profile.require(Capability::DeviceAdministration)?;
        let pending = vault.pending_kex(target_alias)?;
        if pending.source_candidate_id.as_deref() != source_candidate_id
            || expected_user.is_some_and(|user| pending.expected_user.as_deref() != Some(user))
            || (source_candidate_id.is_none() && pending.expected_user.is_some())
        {
            return Err(Error::InvalidAccount(
                "pending KEX source binding does not match",
            ));
        }
        let bound_user: Option<[u8; 33]> = pending
            .expected_user
            .as_deref()
            .map(|user| {
                user.try_into()
                    .map_err(|_| Error::InvalidAccount("pending KEX user binding is invalid"))
            })
            .transpose()?;
        if vault.account_record_exists(target_alias)? {
            let account = vault.account(target_alias)?;
            let expected = derive_device_public(&SecretSeed::new(pending.device_seed))?;
            let actual = account.credential.public_material()?;
            if expected.id != actual.id
                || bound_user
                    .as_ref()
                    .is_some_and(|user| account.credential.uid.as_bytes() != user)
            {
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
        self.finish_kex_acceptance(pending, bound_user.as_ref(), vault)
    }

    fn finish_kex_acceptance(
        &self,
        pending: PendingKexAcceptance,
        expected_user: Option<&[u8; 33]>,
        vault: &mut AccountVault<'_>,
    ) -> Result<DeviceProvisionReport> {
        let host = self.pinned_host()?;
        let provisioned = self.client.accept_kex_provisioning_for_user_within(
            &host,
            &pending.phrase,
            &pending.device_name,
            pending.serial,
            SecretSeed::new(pending.device_seed),
            expected_user,
            self.pairing_budget(),
        )?;
        if expected_user.is_some_and(|user| provisioned.credential.uid.as_bytes() != user) {
            return Err(Error::InvalidAccount(
                "paired account does not match the selected Go profile",
            ));
        }
        let username = String::from_utf8_lossy(provisioned.authenticated.verified.username_utf8())
            .into_owned();
        vault.commit_created(&pending.target_alias, &username, &provisioned.credential)?;
        self.register_default_refresh_jobs_for(&provisioned.credential.uid, now_microseconds()?)?;
        vault.remove_pending_kex(&pending.target_alias)?;
        Ok(DeviceProvisionReport {
            alias: pending.target_alias.clone(),
            device_id_hex: hex(provisioned.credential.public_material()?.id.as_bytes()),
            user_chain_sequence: provisioned.authenticated.verified.chain_seqno(),
        })
    }

    /// Generates an ephemeral backup phrase after validating its intended
    /// account and alias. This does not write local state or contact FOKS.
    pub fn prepare_owner_backup(
        &self,
        account_alias: &str,
        backup_alias: &str,
        vault: &mut AccountVault<'_>,
    ) -> Result<BackupPhrase> {
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
        let _ = vault.account(account_alias)?;
        let backup = BackupKey::generate()?;
        Ok(backup.phrase())
    }

    /// Enrolls a phrase only after the caller has acknowledged retaining it
    /// offline. Ambiguous remote completion is safe to retry with the same
    /// phrase; the local record contains public completion facts only.
    pub fn commit_owner_backup(
        &self,
        account_alias: &str,
        backup_alias: &str,
        phrase: Zeroizing<String>,
        vault: &mut AccountVault<'_>,
    ) -> Result<BackupEnrollmentReport> {
        self.profile.require(Capability::Recovery)?;
        validate_name(backup_alias)?;
        let loaded = vault.account(account_alias)?;
        let backup = BackupKey::from_phrase(&phrase)?;
        let backup_id = backup.public_material()?.id;
        if let Some(existing) = vault.backup(backup_alias)? {
            if existing.account_alias != account_alias || existing.backup_id != backup_id.as_bytes()
            {
                return Err(Error::InvalidAccount(
                    "backup alias is bound to another account or phrase",
                ));
            }
        }
        let host = self.pinned_host()?;
        let enrolled =
            self.client
                .enroll_backup_key(&host, &loaded.credential, Role::OWNER, &backup)?;

        #[cfg(test)]
        if TEST_FAIL_AFTER_BACKUP_ENROLLMENT.swap(false, std::sync::atomic::Ordering::SeqCst) {
            return Err(Error::InvalidAccount(
                "test interrupted backup enrollment before local completion",
            ));
        }

        vault.put_backup(&StoredBackup {
            version: CREDENTIAL_VERSION,
            backup_alias: backup_alias.to_owned(),
            account_alias: account_alias.to_owned(),
            backup_id: backup_id.as_bytes().to_vec(),
        })?;
        Ok(BackupEnrollmentReport {
            backup_alias: backup_alias.to_owned(),
            account_alias: account_alias.to_owned(),
            backup_id_hex: hex(backup_id.as_bytes()),
            user_chain_sequence: enrolled.authenticated.verified.chain_seqno(),
        })
    }

    /// Revokes exactly the locally recorded backup credential. The local
    /// enrollment fact is retained until authenticated state proves the remote
    /// credential absent, making an ambiguous remote completion safe to retry.
    pub fn revoke_owner_backup(
        &self,
        account_alias: &str,
        backup_alias: &str,
        backup_id_hex: &str,
        vault: &mut AccountVault<'_>,
        master_key: &[u8; 32],
    ) -> Result<BackupRevocationReport> {
        self.profile.require(Capability::Recovery)?;
        self.profile.require(Capability::DeviceAdministration)?;
        validate_name(backup_alias)?;
        let signer = vault.account(account_alias)?;
        let target = entity_id_from_hex(backup_id_hex)?;
        target.clone().require_type(foks_proto::ENTITY_BACKUP_KEY)?;
        let stored = vault.backup(backup_alias)?;
        if let Some(stored) = stored.as_ref() {
            if stored.account_alias != account_alias || stored.backup_id != target.as_bytes() {
                return Err(Error::InvalidAccount(
                    "backup revocation does not match the local enrollment binding",
                ));
            }
        }

        let host = self.pinned_host()?;
        let authenticated = self
            .client
            .authenticate_and_pin(&host, &signer.credential)?;
        let target_role = authenticated
            .verified
            .devices()
            .iter()
            .find(|device| device.id == target)
            .map(|device| device.role);
        let already_absent = target_role.is_none();
        if stored.is_none() && !already_absent {
            return Err(Error::InvalidAccount(
                "backup credential is not bound to a local enrollment",
            ));
        }
        let user_chain_sequence = if let Some(target_role) = target_role {
            let (rotations, no_passphrase) = self.software_revocation_material(
                &host,
                &signer.credential,
                &authenticated,
                target_role,
            )?;
            let mut mutations = EncryptedFileMutationStore::open(
                &self.paths.protected_mutations,
                derive_mutation_key(master_key),
            )?;
            self.client
                .revoke_user_credential_with_software_device(
                    &host,
                    &signer.credential,
                    &target,
                    &rotations,
                    no_passphrase,
                    &mut mutations,
                )?
                .verified
                .chain_seqno()
        } else {
            authenticated.verified.chain_seqno()
        };

        #[cfg(test)]
        if TEST_FAIL_AFTER_BACKUP_REVOCATION.swap(false, std::sync::atomic::Ordering::SeqCst) {
            return Err(Error::InvalidAccount(
                "test interrupted backup revocation before local completion",
            ));
        }

        let _ = vault.store.remove(&backup_key(backup_alias))?;
        Ok(BackupRevocationReport {
            backup_alias: backup_alias.to_owned(),
            account_alias: account_alias.to_owned(),
            backup_id_hex: hex(target.as_bytes()),
            user_chain_sequence,
            already_absent,
            removed_local_enrollment: true,
        })
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
            device_id_hex: hex(recovered.credential.public_material()?.id.as_bytes()),
            user_chain_sequence: recovered.authenticated.verified.chain_seqno(),
        })
    }
    pub fn sync_account(&self, alias: &str, vault: &mut AccountVault<'_>) -> Result<SyncReport> {
        self.profile.require(Capability::UserSync)?;
        self.profile.require(Capability::Kv)?;
        let loaded = vault.account(alias)?;
        if loaded.credential.key_kind == foks_client::SoftwareKeyKind::Device {
            self.register_default_refresh_jobs_for(&loaded.credential.uid, now_microseconds()?)?;
        }
        let (_, authenticated, directories) = self.authenticated_tree(alias, vault)?;
        self.refresh_verified_account_labels(&authenticated.verified, vault)?;
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
        // The write already read the committed parcel back and validated it
        // against the argument it signed, so the confirmation here is the
        // server login assertion alone.
        let (metadata, verification) =
            self.client
                .set_passphrase_verified(&host, &loaded.credential, &passphrase)?;
        PassphraseReport::from_verified(metadata, verification)
    }

    /// Reports whether this account has a passphrase on its server, and the
    /// generation it sits at. Callers use this to choose between enrollment
    /// and rotation, which the server accepts under mutually exclusive
    /// conditions.
    pub fn passphrase_status(
        &self,
        alias: &str,
        vault: &mut AccountVault<'_>,
    ) -> Result<PassphraseStatus> {
        self.profile.require(Capability::Passphrases)?;
        let loaded = vault.account(alias)?;
        let host = self.pinned_host()?;
        let generation = self.client.passphrase_status(&host, &loaded.credential)?;
        Ok(PassphraseStatus {
            configured: generation.is_some(),
            generation: generation.unwrap_or_default(),
        })
    }

    /// Rotates the account's passphrase. `current`, when supplied, is checked
    /// against the server first and the rotation is abandoned if it does not
    /// match.
    ///
    /// The rotation itself is authorized by this device's owner PUK and never
    /// needs `current`; passing `None` is the path that recovers an account
    /// whose passphrase has been forgotten. A supplied `current` is a
    /// confirmation step, not an authorization one, and it spends an attempt
    /// against the server's bad-passphrase rate limit when it is wrong.
    pub fn change_passphrase(
        &self,
        alias: &str,
        current: Option<Passphrase>,
        passphrase: Passphrase,
        vault: &mut AccountVault<'_>,
    ) -> Result<PassphraseReport> {
        self.profile.require(Capability::Passphrases)?;
        let loaded = vault.account(alias)?;
        let host = self.pinned_host()?;
        if let Some(current) = current {
            // A rejected check is the caller's own input and belongs on the
            // field that carried it. Every other failure, the server's
            // bad-passphrase rate limit included, stays as it arrived.
            self.client
                .verify_passphrase(&host, &loaded.credential, &current)
                .map_err(|error| match &error {
                    foks_client::Error::Rpc(foks_rpc::Error::RemoteStatus { code, .. })
                        if *code == foks_rpc::STATUS_BAD_PASSPHRASE_ERROR =>
                    {
                        Error::CurrentPassphraseRejected
                    }
                    _ => error.into(),
                })?;
        }
        let (metadata, verification) =
            self.client
                .change_passphrase_verified(&host, &loaded.credential, &passphrase)?;
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

/// Whether an account holds a passphrase, and at which generation.
/// `generation` is zero when none is configured; the server numbers a first
/// enrollment 1.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct PassphraseStatus {
    pub configured: bool,
    pub generation: u64,
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
    pub name: Option<String>,
    pub role: String,
    pub current: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DeviceProvisionReport {
    pub alias: String,
    pub device_id_hex: String,
    pub user_chain_sequence: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DeviceRevocationReport {
    pub device_id_hex: String,
    pub user_chain_sequence: u64,
    pub already_absent: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct BackupEnrollmentReport {
    pub backup_alias: String,
    pub account_alias: String,
    pub backup_id_hex: String,
    pub user_chain_sequence: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct BackupRevocationReport {
    pub backup_alias: String,
    pub account_alias: String,
    pub backup_id_hex: String,
    pub user_chain_sequence: u64,
    pub already_absent: bool,
    pub removed_local_enrollment: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct BackupEnrollmentSummary {
    pub backup_alias: String,
    pub account_alias: String,
    pub backup_id_hex: String,
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
    pub(super) username: String,
    pub(super) device_seed: [u8; 32],
    pub(super) puk_seed: [u8; 32],
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
    source_candidate_id: Option<String>,
    expected_user: Option<Vec<u8>>,
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
    backup_id: Vec<u8>,
}
impl PendingSignup {
    pub(super) fn journal_operation(
        &self,
        host: &foks_client::PinnedHost,
        database: &Path,
    ) -> Result<Option<foks_client_db::MutationOperation>> {
        let mut uid = derive_shared_verify_key(&SecretSeed::new(self.puk_seed), ENTITY_PUK_VERIFY)?
            .into_bytes();
        uid[0] = ENTITY_USER;
        let device = derive_device_public(&SecretSeed::new(self.device_seed))?;
        Ok(HardStateStore::open(database)?.latest_mutation_for_binding(
            host.host_id().as_bytes(),
            MutationKind::Signup,
            device.id.as_bytes(),
            &uid,
        )?)
    }

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

    pub(super) fn secrets(&self) -> Result<SoftwareAccountSecrets> {
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
    pub(super) loaded_bots: BTreeMap<String, LoadedAccount>,
}

impl<'a> AccountVault<'a> {
    pub fn new(store: &'a mut dyn SecretStore) -> Self {
        Self {
            store,
            loaded_bots: BTreeMap::new(),
        }
    }

    pub fn aliases(&mut self) -> Result<Vec<String>> {
        Ok(self
            .store
            .keys()?
            .into_iter()
            .filter_map(|key| {
                key.strip_prefix("account.")
                    .or_else(|| key.strip_prefix("bot-account."))
                    .map(str::to_owned)
            })
            .collect())
    }

    /// Lists only locally persisted public enrollment facts. A record exists
    /// only after remote enrollment completed; recovery phrases are never
    /// stored and therefore cannot be returned here.
    pub fn backup_enrollments(
        &mut self,
        account_alias: &str,
    ) -> Result<Vec<BackupEnrollmentSummary>> {
        validate_name(account_alias)?;
        let mut summaries = Vec::new();
        for key in self.store.keys()? {
            let Some(alias) = key.strip_prefix("backup.") else {
                continue;
            };
            validate_name(alias)?;
            let backup = self.backup(alias)?.ok_or(Error::InvalidAccount(
                "backup enrollment disappeared during enumeration",
            ))?;
            if backup.account_alias == account_alias {
                summaries.push(BackupEnrollmentSummary {
                    backup_alias: backup.backup_alias,
                    account_alias: backup.account_alias,
                    backup_id_hex: hex(&backup.backup_id),
                });
            }
        }
        summaries.sort_by(|left, right| left.backup_alias.cmp(&right.backup_alias));
        Ok(summaries)
    }

    /// Enumerates resumable product operations without exposing their
    /// protected material. Reading each record authenticates it before a UI
    /// is allowed to advertise a Resume action.
    pub fn pending_operations(&mut self) -> Result<Vec<PendingOperationSummary>> {
        const PREFIXES: [(&str, PendingOperationKind); 9] = [
            ("pending.", PendingOperationKind::AccountSignup),
            ("pending-device.", PendingOperationKind::DeviceProvision),
            ("kex-offer.", PendingOperationKind::PairingOffer),
            ("pending-kex.", PendingOperationKind::PairingAcceptance),
            ("pending-recovery.", PendingOperationKind::AccountRecovery),
            ("pending-yubi.", PendingOperationKind::YubiEnrollment),
            ("team-member-edit.", PendingOperationKind::TeamMemberEdit),
            (
                "federation-expulsion.",
                PendingOperationKind::FederationExpulsion,
            ),
            ("team-rekey.", PendingOperationKind::TeamRekey),
        ];
        let mut operations = Vec::new();
        for key in self.store.keys()? {
            let Some((prefix, kind)) = PREFIXES.iter().find(|(prefix, _)| key.starts_with(prefix))
            else {
                continue;
            };
            let alias = key
                .strip_prefix(prefix)
                .ok_or(Error::InvalidAccount("pending operation key changed"))?;
            validate_name(alias)?;
            match kind {
                PendingOperationKind::AccountSignup => drop(self.pending(alias)?),
                PendingOperationKind::DeviceProvision => drop(self.pending_device(alias)?),
                PendingOperationKind::PairingOffer => drop(self.kex_offer(alias)?),
                PendingOperationKind::PairingAcceptance => drop(self.pending_kex(alias)?),
                PendingOperationKind::AccountRecovery => drop(self.pending_recovery(alias)?),
                PendingOperationKind::YubiEnrollment => self.validate_pending_yubi_record(alias)?,
                PendingOperationKind::TeamMemberEdit => {
                    self.team_member_edit(alias)?.ok_or(Error::InvalidAccount(
                        "pending team member edit disappeared during enumeration",
                    ))?;
                }
                PendingOperationKind::FederationExpulsion => {
                    self.federation_expulsion(alias)?
                        .ok_or(Error::InvalidAccount(
                            "pending federation expulsion disappeared during enumeration",
                        ))?;
                }
                PendingOperationKind::TeamRekey => {
                    self.team_rekey(alias)?.ok_or(Error::InvalidAccount(
                        "pending team rekey disappeared during enumeration",
                    ))?;
                }
                PendingOperationKind::TeamCreation | PendingOperationKind::TeamMemberAddition => {
                    return Err(Error::InvalidAccount(
                        "derived team operation has no direct pending record",
                    ));
                }
            }
            operations.push(PendingOperationSummary {
                kind: *kind,
                alias: alias.to_owned(),
                target: None,
            });
        }
        for alias in self.team_aliases()? {
            let team = self.team(&alias)?;
            if !team.active {
                operations.push(PendingOperationSummary {
                    kind: PendingOperationKind::TeamCreation,
                    alias: alias.clone(),
                    target: None,
                });
            }
            operations.extend(
                team.local_members
                    .iter()
                    .filter(|member| !member.active)
                    .map(|member| PendingOperationSummary {
                        kind: PendingOperationKind::TeamMemberAddition,
                        alias: alias.clone(),
                        target: Some(member.username.clone()),
                    }),
            );
        }
        operations.sort_by(|left, right| {
            left.alias
                .cmp(&right.alias)
                .then_with(|| left.kind.cmp(&right.kind))
                .then_with(|| left.target.cmp(&right.target))
        });
        Ok(operations)
    }

    pub fn contains(&mut self, alias: &str) -> Result<bool> {
        validate_name(alias)?;
        Ok(self.store.keys()?.iter().any(|key| {
            key == &account_key(alias)
                || key == &format!("bot-account.{alias}")
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
            .any(|key| key == &account_key(alias) || key == &format!("bot-account.{alias}")))
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
        if self.bot_selection(alias)?.is_some() {
            return self.loaded_bot(alias);
        }
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
                key_kind: foks_client::SoftwareKeyKind::Device,
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

    fn backup(&mut self, backup_alias: &str) -> Result<Option<StoredBackup>> {
        validate_name(backup_alias)?;
        let bytes = match self.store.get(&backup_key(backup_alias)) {
            Ok(bytes) => bytes,
            Err(foks_keystore::Error::Missing) => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let backup: StoredBackup = serde_json::from_slice(&bytes)?;
        validate_stored_backup(&backup, backup_alias)?;
        Ok(Some(backup))
    }

    fn put_backup(&mut self, backup: &StoredBackup) -> Result<()> {
        validate_stored_backup(backup, &backup.backup_alias)?;
        let encoded = Zeroizing::new(serde_json::to_vec(backup)?);
        self.store
            .put(&backup_key(&backup.backup_alias), &encoded)?;
        Ok(())
    }

    pub(super) fn refresh_uid_labels(&mut self, uid: &EntityId, username: &[u8]) -> Result<()> {
        let name = std::str::from_utf8(username)
            .map_err(|_| Error::InvalidAccount("verified username is not UTF8"))?;
        for alias in self.aliases()? {
            if let Some(bot) = self.bot_selection(&alias)? {
                if bot.uid == uid.as_bytes() {
                    self.update_bot_label(&alias, name)?;
                }
                continue;
            }
            let account = self.account(&alias)?;
            if account.credential.uid == *uid && account.username != name {
                self.commit_created(&alias, name, &account.credential)?;
            }
        }
        self.refresh_yubi_uid_labels(uid, name)
    }

    pub(super) fn commit_created(
        &mut self,
        alias: &str,
        username: &str,
        credential: &DeviceCredential,
    ) -> Result<()> {
        if credential.key_kind != foks_client::SoftwareKeyKind::Device {
            return Err(Error::InvalidAccount(
                "bot secrets cannot enter the persistent account vault",
            ));
        }
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
        return Err(Error::InvalidAccount(
            "credential version or alias does not match",
        ));
    }
    validate_name(&account.alias)?;
    if account.username.is_empty() || account.username.len() > 256 {
        return Err(Error::InvalidAccount(
            "username is empty or exceeds 256 bytes",
        ));
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
        || pending.source_candidate_id.is_some() != pending.expected_user.is_some()
        || pending.source_candidate_id.as_ref().is_some_and(|id| {
            id.len() != 64
                || !id
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        })
        || pending
            .expected_user
            .as_ref()
            .is_some_and(|user| user.len() != 33 || user[0] != 1)
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

fn validate_stored_backup(backup: &StoredBackup, expected_alias: &str) -> Result<()> {
    if backup.version != CREDENTIAL_VERSION || backup.backup_alias != expected_alias {
        return Err(Error::InvalidAccount(
            "backup version or alias binding changed",
        ));
    }
    validate_name(&backup.backup_alias)?;
    validate_name(&backup.account_alias)?;
    EntityId::from_bytes(backup.backup_id.clone())?.require_type(foks_proto::ENTITY_BACKUP_KEY)?;
    Ok(())
}

pub(super) fn validate_certificates(certificates: &[Vec<u8>]) -> Result<()> {
    if certificates.is_empty()
        || certificates.len() > MAX_CERTIFICATES
        || certificates
            .iter()
            .any(|certificate| certificate.is_empty() || certificate.len() > MAX_CERTIFICATE_BYTES)
    {
        return Err(Error::InvalidAccount(
            "certificate chain is empty, exceeds certificate limit, or contains oversized certificates",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use foks_keystore::EncryptedFileSecretStore;
    use foks_server_testkit::TestEnvironment;

    struct InterruptAccountCommit {
        inner: EncryptedFileSecretStore,
        interrupt: bool,
    }

    impl SecretStore for InterruptAccountCommit {
        fn put(&mut self, key: &str, value: &[u8]) -> foks_keystore::Result<()> {
            if key.starts_with("account.") && std::mem::take(&mut self.interrupt) {
                return Err(std::io::Error::other("interrupted credential commit").into());
            }
            self.inner.put(key, value)
        }

        fn get(&mut self, key: &str) -> foks_keystore::Result<Zeroizing<Vec<u8>>> {
            self.inner.get(key)
        }

        fn remove(&mut self, key: &str) -> foks_keystore::Result<bool> {
            self.inner.remove(key)
        }

        fn keys(&mut self) -> foks_keystore::Result<Vec<String>> {
            self.inner.keys()
        }
    }

    #[test]
    fn onboarding_resumes_committed_signup_and_device_without_replacing_keys() {
        let environment = TestEnvironment::new().unwrap();
        let _server = environment.start_server().unwrap();
        let state = environment
            .client_path("onboarding-resume", "state")
            .unwrap();
        let root = environment
            .client_path("onboarding-resume", "probe.der")
            .unwrap();
        environment.write_probe_root(&root).unwrap();
        let credentials =
            ClientCredentials::initialize(&state, CredentialBackend::PrivateFile).unwrap();
        let master = credentials.master_key().unwrap();
        let mut registry = ProfileRegistry::open(&state).unwrap();
        registry
            .add(Profile {
                name: "local".to_owned(),
                label: None,
                probe: format!(
                    "localhost:{}",
                    environment.addresses().unwrap().probe.port()
                ),
                protocol: ProtocolPolicy::V019,
                trust: TrustRoot::CertificateDer { path: root },
            })
            .unwrap();
        let session = ProfileSession::open(&registry, "local").unwrap();

        credentials
            .with_checked_session(&session, |session| {
                let mut store = EncryptedFileSecretStore::open(
                    &session.paths().credential_store,
                    derive_vault_key(&master),
                )?;
                let mut vault = AccountVault::new(&mut store);
                // A missing probe must not reserve an alias at all.
                assert!(session
                    .create_account(
                        "owner",
                        "resumeowner",
                        "laptop",
                        "",
                        "",
                        None,
                        &mut vault,
                        &master
                    )
                    .is_err());
                assert!(!vault.contains("owner")?);
                session.probe_and_pin()?;
                // A failed server preflight must also leave the alias reusable.
                assert!(session
                    .create_account("owner", "!", "laptop", "", "", None, &mut vault, &master)
                    .is_err());
                assert!(!vault.contains("owner")?);
                // Simulate a process exit between storing seeds and journaling.
                vault.put_pending(&PendingSignup::random("owner", "resumeowner")?)?;
                let error = session
                    .resume_account("owner", &mut vault, &master)
                    .unwrap_err();
                assert!(error.to_string().contains("create the account again"));
                assert!(!vault.contains("owner")?);
                Ok::<_, Error>(())
            })
            .unwrap();

        let pending_device_id = credentials
            .with_checked_session(&session, |session| {
                let mut store = InterruptAccountCommit {
                    inner: EncryptedFileSecretStore::open(
                        &session.paths().credential_store,
                        derive_vault_key(&master),
                    )?,
                    interrupt: true,
                };
                let mut vault = AccountVault::new(&mut store);
                let error = session
                    .create_account(
                        "owner",
                        "resumeowner",
                        "laptop",
                        "",
                        "",
                        None,
                        &mut vault,
                        &master,
                    )
                    .unwrap_err();
                assert!(error.to_string().contains("interrupted credential commit"));
                assert!(vault.contains("owner")?);
                assert!(!vault.account_record_exists("owner")?);
                let pending = vault.pending("owner")?;
                Ok::<_, Error>(derive_device_public(&SecretSeed::new(pending.device_seed))?.id)
            })
            .unwrap();

        // Reopen protected storage to simulate process restart.
        credentials
            .with_checked_session(&session, |session| {
                let mut store = EncryptedFileSecretStore::open(
                    &session.paths().credential_store,
                    derive_vault_key(&master),
                )?;
                let mut vault = AccountVault::new(&mut store);
                session.resume_account("owner", &mut vault, &master)?;
                assert_eq!(
                    derive_device_public(&vault.account("owner")?.credential.seed)?.id,
                    pending_device_id
                );
                assert!(matches!(vault.pending("owner"), Err(Error::AccountMissing)));
                session.resume_account("owner", &mut vault, &master)?;
                assert_eq!(session.list_devices("owner", &mut vault)?.len(), 1);
                let pending = PendingDevice::random("owner", "second", "resumeowner", 2)?;
                vault.put_pending_device(&pending)?;
                let error = session
                    .resume_owner_device_provision("second", &mut vault, &master)
                    .unwrap_err();
                assert!(error.to_string().contains("provision again"));
                assert!(!vault.contains("second")?);
                Ok::<_, Error>(())
            })
            .unwrap();

        let second_id = credentials
            .with_checked_session(&session, |session| {
                let mut store = InterruptAccountCommit {
                    inner: EncryptedFileSecretStore::open(
                        &session.paths().credential_store,
                        derive_vault_key(&master),
                    )?,
                    interrupt: true,
                };
                let mut vault = AccountVault::new(&mut store);
                let error = session
                    .provision_owner_device(
                        "owner",
                        "second",
                        "second laptop",
                        2,
                        &mut vault,
                        &master,
                    )
                    .unwrap_err();
                assert!(error.to_string().contains("interrupted credential commit"));
                let pending = vault.pending_device("second")?;
                Ok::<_, Error>(derive_device_public(&SecretSeed::new(pending.device_seed))?.id)
            })
            .unwrap();

        credentials
            .with_checked_session(&session, |session| {
                let mut store = EncryptedFileSecretStore::open(
                    &session.paths().credential_store,
                    derive_vault_key(&master),
                )?;
                let mut vault = AccountVault::new(&mut store);
                session.resume_owner_device_provision("second", &mut vault, &master)?;
                assert_eq!(
                    derive_device_public(&vault.account("second")?.credential.seed)?.id,
                    second_id
                );
                assert!(matches!(
                    vault.pending_device("second"),
                    Err(Error::AccountMissing)
                ));
                session.resume_owner_device_provision("second", &mut vault, &master)?;
                assert_eq!(session.list_devices("owner", &mut vault)?.len(), 2);
                Ok::<_, Error>(())
            })
            .unwrap();
    }

    #[test]
    fn backup_prepare_is_ephemeral_and_commit_reconciles_after_interruption() {
        let environment = TestEnvironment::new().unwrap();
        let _server = environment.start_server().unwrap();
        let addresses = environment.addresses().unwrap();
        let state = environment.client_path("backup-split", "state").unwrap();
        let root = environment
            .client_path("backup-split", "probe-root.der")
            .unwrap();
        environment.write_probe_root(&root).unwrap();
        let credentials =
            ClientCredentials::initialize(&state, CredentialBackend::PrivateFile).unwrap();
        let master = credentials.master_key().unwrap();
        let mut registry = ProfileRegistry::open(&state).unwrap();
        registry
            .add(Profile {
                name: "local".to_owned(),
                label: None,
                probe: format!("localhost:{}", addresses.probe.port()),
                protocol: ProtocolPolicy::V019,
                trust: TrustRoot::CertificateDer { path: root },
            })
            .unwrap();
        let session = ProfileSession::open(&registry, "local").unwrap();
        credentials
            .with_checked_session(&session, |session| {
                session.probe_and_pin()?;
                let mut store = EncryptedFileSecretStore::open(
                    &session.paths().credential_store,
                    derive_vault_key(&master),
                )?;
                session.create_account(
                    "personal",
                    "backupowner",
                    "owner laptop",
                    "owner@example.test",
                    "",
                    None,
                    &mut AccountVault::new(&mut store),
                    &master,
                )?;
                Ok::<_, Error>(())
            })
            .unwrap();

        let phrase = credentials
            .with_checked_session(&session, |session| {
                let mut store = EncryptedFileSecretStore::open(
                    &session.paths().credential_store,
                    derive_vault_key(&master),
                )?;
                let mut vault = AccountVault::new(&mut store);
                let initial_devices = session.list_devices("personal", &mut vault)?;
                assert_eq!(initial_devices.len(), 1);
                assert_eq!(initial_devices[0].name.as_deref(), Some("owner laptop"));
                assert!(initial_devices[0].current);
                let before = initial_devices.len();
                let discarded =
                    session.prepare_owner_backup("personal", "discarded", &mut vault)?;
                assert_eq!(format!("{discarded:?}"), "BackupPhrase([REDACTED])");
                assert!(vault.backup("discarded")?.is_none());
                drop(discarded);
                assert_eq!(session.list_devices("personal", &mut vault)?.len(), before);

                let phrase = session.prepare_owner_backup("personal", "paper", &mut vault)?;
                let joined = phrase.expose_joined();
                TEST_FAIL_AFTER_BACKUP_ENROLLMENT.store(true, std::sync::atomic::Ordering::SeqCst);
                session
                    .commit_owner_backup(
                        "personal",
                        "paper",
                        Zeroizing::new(joined.as_str().to_owned()),
                        &mut vault,
                    )
                    .expect_err("the failpoint interrupts after remote enrollment");
                assert!(vault.backup("paper")?.is_none());
                assert_eq!(
                    session.list_devices("personal", &mut vault)?.len(),
                    before + 1
                );
                Ok::<_, Error>(joined)
            })
            .unwrap();

        credentials
            .with_checked_session(&session, |session| {
                let mut store = EncryptedFileSecretStore::open(
                    &session.paths().credential_store,
                    derive_vault_key(&master),
                )?;
                let mut vault = AccountVault::new(&mut store);
                let completed = session.commit_owner_backup(
                    "personal",
                    "paper",
                    Zeroizing::new(phrase.as_str().to_owned()),
                    &mut vault,
                )?;
                assert_eq!(completed.backup_alias, "paper");
                assert_eq!(completed.account_alias, "personal");
                assert_eq!(completed.backup_id_hex.len(), 66);
                assert_eq!(session.list_devices("personal", &mut vault)?.len(), 2);

                let encoded = vault.store.get(&backup_key("paper"))?;
                let value: serde_json::Value = serde_json::from_slice(&encoded)?;
                assert!(value.get("phrase").is_none());
                assert!(value
                    .get("backup_id")
                    .and_then(serde_json::Value::as_array)
                    .is_some());
                assert!(!String::from_utf8_lossy(&encoded).contains(phrase.as_str()));

                let repeated = session.commit_owner_backup(
                    "personal",
                    "paper",
                    Zeroizing::new(phrase.as_str().to_owned()),
                    &mut vault,
                )?;
                assert_eq!(repeated, completed);
                assert_eq!(session.list_devices("personal", &mut vault)?.len(), 2);
                assert_eq!(
                    vault.backup_enrollments("personal")?,
                    vec![BackupEnrollmentSummary {
                        backup_alias: completed.backup_alias.clone(),
                        account_alias: completed.account_alias.clone(),
                        backup_id_hex: completed.backup_id_hex.clone(),
                    }]
                );
                assert!(vault.backup_enrollments("missing")?.is_empty());

                let other_phrase = session
                    .prepare_owner_backup("personal", "other-paper", &mut vault)?
                    .expose_joined();
                assert!(session
                    .commit_owner_backup(
                        "personal",
                        "paper",
                        Zeroizing::new(other_phrase.as_str().to_owned()),
                        &mut vault,
                    )
                    .is_err());
                assert_eq!(session.list_devices("personal", &mut vault)?.len(), 2);

                let other_id = format!("10{}", "55".repeat(32));
                assert!(session
                    .revoke_owner_backup("personal", "paper", &other_id, &mut vault, &master,)
                    .is_err());
                assert!(vault.backup("paper")?.is_some());
                assert_eq!(session.list_devices("personal", &mut vault)?.len(), 2);

                TEST_FAIL_AFTER_BACKUP_REVOCATION.store(true, std::sync::atomic::Ordering::SeqCst);
                session
                    .revoke_owner_backup(
                        "personal",
                        "paper",
                        &completed.backup_id_hex,
                        &mut vault,
                        &master,
                    )
                    .expect_err("the failpoint interrupts before local cleanup");
                assert!(vault.backup("paper")?.is_some());
                assert_eq!(session.list_devices("personal", &mut vault)?.len(), 1);

                let reconciled = session.revoke_owner_backup(
                    "personal",
                    "paper",
                    &completed.backup_id_hex,
                    &mut vault,
                    &master,
                )?;
                assert!(reconciled.already_absent);
                assert!(reconciled.removed_local_enrollment);
                assert!(vault.backup("paper")?.is_none());
                assert!(vault.backup_enrollments("personal")?.is_empty());

                let repeated = session.revoke_owner_backup(
                    "personal",
                    "paper",
                    &completed.backup_id_hex,
                    &mut vault,
                    &master,
                )?;
                assert!(repeated.already_absent);
                assert!(repeated.removed_local_enrollment);
                assert_eq!(repeated.user_chain_sequence, reconciled.user_chain_sequence);
                Ok::<_, Error>(())
            })
            .unwrap();
    }

    #[test]
    fn imported_software_device_is_reauthenticated_and_idempotent() {
        let environment = TestEnvironment::new().unwrap();
        let _server = environment.start_server().unwrap();
        let addresses = environment.addresses().unwrap();
        let root = environment
            .client_path("import-copy", "probe-root.der")
            .unwrap();
        environment.write_probe_root(&root).unwrap();
        let profile = Profile {
            name: "local".to_owned(),
            label: None,
            probe: format!("localhost:{}", addresses.probe.port()),
            protocol: ProtocolPolicy::V019,
            trust: TrustRoot::CertificateDer { path: root },
        };

        let source_state = environment
            .client_path("import-copy-source", "state")
            .unwrap();
        let source_credentials =
            ClientCredentials::initialize(&source_state, CredentialBackend::PrivateFile).unwrap();
        let source_master = source_credentials.master_key().unwrap();
        let mut source_registry = ProfileRegistry::open(&source_state).unwrap();
        source_registry.add(profile.clone()).unwrap();
        let source_session = ProfileSession::open(&source_registry, "local").unwrap();
        let (uid, device_id, seed) = source_credentials
            .with_checked_session(&source_session, |session| {
                session.probe_and_pin()?;
                let mut store = EncryptedFileSecretStore::open(
                    &session.paths().credential_store,
                    derive_vault_key(&source_master),
                )?;
                let mut vault = AccountVault::new(&mut store);
                session.create_account(
                    "source",
                    "importowner",
                    "source laptop",
                    "",
                    "",
                    None,
                    &mut vault,
                    &source_master,
                )?;
                let loaded = vault.account("source")?;
                let uid: [u8; 33] = loaded.credential.uid.as_bytes().try_into().unwrap();
                let device = loaded.credential.public_material()?;
                let device_id: [u8; 33] = device.id.as_bytes().try_into().unwrap();
                Ok::<_, Error>((uid, device_id, *loaded.credential.seed.as_bytes()))
            })
            .unwrap();

        let destination_state = environment
            .client_path("import-copy-destination", "state")
            .unwrap();
        let destination_credentials =
            ClientCredentials::initialize(&destination_state, CredentialBackend::PrivateFile)
                .unwrap();
        let destination_master = destination_credentials.master_key().unwrap();
        let mut destination_registry = ProfileRegistry::open(&destination_state).unwrap();
        destination_registry.add(profile).unwrap();
        let destination_session = ProfileSession::open(&destination_registry, "local").unwrap();
        destination_credentials
            .with_checked_session(&destination_session, |session| {
                session.probe_and_pin()?;
                let mut store = EncryptedFileSecretStore::open(
                    &session.paths().credential_store,
                    derive_vault_key(&destination_master),
                )?;
                let mut vault = AccountVault::new(&mut store);
                let first = session.import_software_device(
                    "imported",
                    &uid,
                    &device_id,
                    SecretSeed::new(seed),
                    &mut vault,
                )?;
                let repeated = session.import_software_device(
                    "imported",
                    &uid,
                    &device_id,
                    SecretSeed::new(seed),
                    &mut vault,
                )?;
                assert_eq!(first, repeated);
                assert_eq!(vault.aliases()?, vec!["imported"]);
                assert_eq!(vault.account("imported")?.username, "importowner");
                Ok::<_, Error>(())
            })
            .unwrap();
    }

    #[test]
    fn backup_enrollment_list_rejects_authenticated_malformed_records() {
        let mut store = foks_keystore::MemorySecretStore::default();
        foks_keystore::SecretStore::put(
            &mut store,
            &backup_key("paper"),
            b"authenticated but malformed",
        )
        .unwrap();
        assert!(AccountVault::new(&mut store)
            .backup_enrollments("personal")
            .is_err());
    }
}

mod inventory;
pub(crate) use inventory::{validate_archive_vault_key, VaultRecordDescriptor};
