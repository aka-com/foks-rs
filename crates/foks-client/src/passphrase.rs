//! Passphrase enrollment, rotation, and public challenge verification.

pub use foks_crypto::Passphrase;
use foks_crypto::{
    change_passphrase_with_puk, create_passphrase_enrollment, prepare_passphrase_login,
};
use foks_proto::{
    decode_stretch_version, GenericLinkPayload, PassphraseInfo, PassphraseLoginResult,
    PassphraseUpdateArgument, PostGenericLinkArgument, PpeParcel, RegistrationChallenge,
    StretchVersion, LINK_OUTER_TYPE_ID,
};
use foks_rpc::{
    encode_change_passphrase_request, encode_get_login_challenge_request,
    encode_get_ppe_parcel_request, encode_passphrase_login_request,
    encode_registration_select_vhost_request, encode_registration_stretch_version_request,
    encode_set_passphrase_request, encode_user_stretch_version_request,
};

use crate::{
    current_owner_puk, DeviceCredential, Error, FoksClient, PinnedHost, Result, YubiCredential,
};

const LOCAL_PPE_PARCEL_HASH_TYPE_ID: u64 = 0xf06c_dce5_31a0_8b42;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PassphraseMetadata {
    pub salt: [u8; 16],
    pub generation: u64,
    pub stretch_version: StretchVersion,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PassphraseVerification {
    pub generation: u64,
}

#[derive(Clone, Copy)]
pub(crate) enum PassphrasePukTarget<'a> {
    CurrentOwner,
    FutureOwner {
        previous_seed: &'a foks_proto::SecretSeed,
        new_seed: &'a foks_proto::SecretSeed,
        new_generation: u64,
    },
}

impl FoksClient {
    pub(crate) fn kex_passphrase_package(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
        authenticated: &crate::AuthenticatedUserOutcome,
        owner_puk: &foks_proto::SecretSeed,
    ) -> Result<Option<foks_proto::KexPpe>> {
        let (_, settings, _) =
            self.authenticated_user_settings(host, credential, &authenticated.verified)?;
        let parcel = match self.fetch_ppe_parcel(host, credential) {
            Ok(parcel) => parcel,
            Err(Error::Rpc(foks_rpc::Error::RemoteStatus {
                code: foks_rpc::STATUS_PASSPHRASE_NOT_FOUND_ERROR,
                ..
            })) if settings.is_none() => return Ok(None),
            Err(Error::Rpc(foks_rpc::Error::RemoteStatus {
                code: foks_rpc::STATUS_PASSPHRASE_NOT_FOUND_ERROR,
                ..
            })) => {
                return Err(Error::CredentialBinding(
                    "server omitted PPE state committed by user settings",
                ));
            }
            Err(error) => return Err(error),
        };
        let settings = settings.ok_or(Error::CredentialBinding(
            "server returned PPE without authenticated user settings",
        ))?;
        if settings.generation != parcel.generation
            || settings.salt != Some(parcel.salt)
            || settings.stretch_version != parcel.stretch_version.protocol_value()
        {
            return Err(Error::CredentialBinding(
                "authenticated settings and KEX passphrase parcel diverge",
            ));
        }
        Ok(Some(foks_crypto::export_kex_ppe(
            &credential.uid,
            host.host_id(),
            &parcel,
            owner_puk,
        )?))
    }

    pub fn authenticated_passphrase_settings(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
        _authenticated: &crate::AuthenticatedUserOutcome,
    ) -> Result<Option<PassphraseInfo>> {
        self.retry_chain_load(host, |current_host| {
            let authenticated = self.authenticate_and_pin(current_host, credential)?;
            let (_, settings, _) = self.authenticated_user_settings(
                current_host,
                credential,
                &authenticated.verified,
            )?;
            Ok(settings)
        })
    }

    pub fn set_passphrase(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
        passphrase: &Passphrase,
    ) -> Result<PassphraseMetadata> {
        let authenticated = self.authenticate_and_pin(host, credential)?;
        let owner = current_owner_puk(&authenticated)?;
        let stretch = self.authenticated_stretch_version(host, credential)?;
        let update = create_passphrase_enrollment(
            passphrase,
            &credential.uid,
            host.host_id(),
            &owner.seed,
            owner.generation,
            stretch,
        )?;
        let argument = self.with_user_settings_link(
            host,
            credential,
            &authenticated,
            update.argument(),
            None,
            PassphrasePukTarget::CurrentOwner,
        )?;
        // Enrollment establishes the first passphrase (generation 1). The Go
        // v0.1.9 server never assigns nextPassphraseGeneration, so gate on the
        // locally derived generation rather than that RPC; the server still
        // rejects a set when a passphrase already exists.
        if argument.generation != 1 {
            return Err(Error::CredentialBinding(
                "passphrase enrollment must be generation 1",
            ));
        }
        let stored = self.submit_passphrase_update(
            host,
            credential,
            encode_set_passphrase_request(&argument)?,
            &argument,
        )?;
        Ok(metadata(&stored))
    }

    pub fn set_passphrase_yubi(
        &self,
        host: &PinnedHost,
        credential: &YubiCredential<'_>,
        passphrase: &Passphrase,
    ) -> Result<PassphraseMetadata> {
        let authenticated = self.authenticate_yubi_and_pin(host, credential)?;
        let owner = current_owner_puk(&authenticated)?;
        let stretch = self.authenticated_stretch_version_yubi(host, credential)?;
        let update = create_passphrase_enrollment(
            passphrase,
            &credential.uid,
            host.host_id(),
            &owner.seed,
            owner.generation,
            stretch,
        )?;
        let argument = self.with_user_settings_link_yubi(
            host,
            credential,
            &authenticated,
            update.argument(),
            None,
            PassphrasePukTarget::CurrentOwner,
        )?;
        // Enrollment establishes the first passphrase (generation 1). The Go
        // v0.1.9 server never assigns nextPassphraseGeneration, so gate on the
        // locally derived generation rather than that RPC; the server still
        // rejects a set when a passphrase already exists.
        if argument.generation != 1 {
            return Err(Error::CredentialBinding(
                "passphrase enrollment must be generation 1",
            ));
        }
        let stored = self.submit_passphrase_update_yubi(
            host,
            credential,
            encode_set_passphrase_request(&argument)?,
            &argument,
        )?;
        Ok(metadata(&stored))
    }

    pub fn change_passphrase(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
        new_passphrase: &Passphrase,
    ) -> Result<PassphraseMetadata> {
        let authenticated = self.authenticate_and_pin(host, credential)?;
        let owner = current_owner_puk(&authenticated)?;
        let current = self.fetch_ppe_parcel(host, credential)?;
        // The next generation is the fetched parcel's generation plus one. The Go
        // v0.1.9 server never assigns nextPassphraseGeneration, so derive it locally
        // rather than calling that RPC; a concurrent change is caught by the server
        // when this update is submitted against a now-stale generation.
        let next = current
            .generation
            .checked_add(1)
            .ok_or(Error::CredentialBinding("passphrase generation overflow"))?;
        let update = change_passphrase_with_puk(
            new_passphrase,
            &credential.uid,
            host.host_id(),
            &current,
            &owner.seed,
            &owner.seed,
            owner.generation,
        )?;
        let argument = self.with_user_settings_link(
            host,
            credential,
            &authenticated,
            update.argument(),
            Some(&current),
            PassphrasePukTarget::CurrentOwner,
        )?;
        if argument.generation != next {
            return Err(Error::CredentialBinding(
                "local passphrase generation does not match the server",
            ));
        }
        let stored = self.submit_passphrase_update(
            host,
            credential,
            encode_change_passphrase_request(&argument)?,
            &argument,
        )?;
        Ok(metadata(&stored))
    }

    pub fn change_passphrase_yubi(
        &self,
        host: &PinnedHost,
        credential: &YubiCredential<'_>,
        new_passphrase: &Passphrase,
    ) -> Result<PassphraseMetadata> {
        let authenticated = self.authenticate_yubi_and_pin(host, credential)?;
        let owner = current_owner_puk(&authenticated)?;
        let current = self.fetch_ppe_parcel_yubi(host, credential)?;
        // The next generation is the fetched parcel's generation plus one. The Go
        // v0.1.9 server never assigns nextPassphraseGeneration, so derive it locally
        // rather than calling that RPC; a concurrent change is caught by the server
        // when this update is submitted against a now-stale generation.
        let next = current
            .generation
            .checked_add(1)
            .ok_or(Error::CredentialBinding("passphrase generation overflow"))?;
        let update = change_passphrase_with_puk(
            new_passphrase,
            &credential.uid,
            host.host_id(),
            &current,
            &owner.seed,
            &owner.seed,
            owner.generation,
        )?;
        let argument = self.with_user_settings_link_yubi(
            host,
            credential,
            &authenticated,
            update.argument(),
            Some(&current),
            PassphrasePukTarget::CurrentOwner,
        )?;
        if argument.generation != next {
            return Err(Error::CredentialBinding(
                "local passphrase generation does not match the server",
            ));
        }
        let stored = self.submit_passphrase_update_yubi(
            host,
            credential,
            encode_change_passphrase_request(&argument)?,
            &argument,
        )?;
        Ok(metadata(&stored))
    }

    /// Performs the public v0.1.9 login challenge and opens the returned PPE
    /// history locally. The active device is used only to obtain the user's
    /// encrypted parcel metadata; the login itself proves the passphrase key.
    pub fn verify_passphrase(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
        passphrase: &Passphrase,
    ) -> Result<PassphraseVerification> {
        self.retry_chain_load(host, |current_host| {
            let authenticated = self.authenticate_and_pin(current_host, credential)?;
            let (_, settings, _) = self.authenticated_user_settings(
                current_host,
                credential,
                &authenticated.verified,
            )?;
            let parcel = self.fetch_ppe_parcel(current_host, credential)?;
            require_settings_match_parcel(settings.as_ref(), &parcel)?;
            let verified = self.verify_passphrase_against_parcel(
                current_host,
                &credential.uid,
                passphrase,
                &parcel,
            )?;
            let trusted = self.trust_passphrase_parcel_if_safe(
                current_host,
                &credential.uid,
                &authenticated,
                &parcel,
            )?;
            if settings.is_none() && trusted {
                self.bootstrap_user_settings_from_signup(
                    current_host,
                    credential,
                    &passphrase_argument(&parcel),
                )?;
            }
            Ok(verified)
        })
    }

    pub fn verify_passphrase_yubi(
        &self,
        host: &PinnedHost,
        credential: &YubiCredential<'_>,
        passphrase: &Passphrase,
    ) -> Result<PassphraseVerification> {
        self.retry_chain_load(host, |current_host| {
            let authenticated = self.authenticate_yubi_and_pin(current_host, credential)?;
            let (_, settings, _) = self.authenticated_user_settings_yubi(
                current_host,
                credential,
                &authenticated.verified,
            )?;
            let parcel = self.fetch_ppe_parcel_yubi(current_host, credential)?;
            require_settings_match_parcel(settings.as_ref(), &parcel)?;
            let verified = self.verify_passphrase_against_parcel(
                current_host,
                &credential.uid,
                passphrase,
                &parcel,
            )?;
            let trusted = self.trust_passphrase_parcel_if_safe(
                current_host,
                &credential.uid,
                &authenticated,
                &parcel,
            )?;
            if settings.is_none() && trusted {
                self.bootstrap_user_settings_from_signup_yubi(
                    current_host,
                    credential,
                    &passphrase_argument(&parcel),
                )?;
            }
            Ok(verified)
        })
    }

    pub(crate) fn bootstrap_user_settings_from_signup(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
        expected: &PassphraseUpdateArgument,
    ) -> Result<()> {
        let stored = self.fetch_ppe_parcel(host, credential)?;
        validate_committed_update(&stored, expected)?;
        self.trust_passphrase_parcel(host, &credential.uid, &stored)?;
        let authenticated = self.authenticate_and_pin(host, credential)?;
        let (tail, current, _) =
            self.authenticated_user_settings(host, credential, &authenticated.verified)?;
        let info = PassphraseInfo {
            generation: expected.generation,
            salt: Some(expected.salt),
            stretch_version: expected.stretch_version.protocol_value(),
        };
        if let Some(current) = current {
            return if current == info {
                Ok(())
            } else {
                Err(Error::CredentialBinding(
                    "signup passphrase conflicts with authenticated user settings",
                ))
            };
        }
        let link =
            self.make_user_settings_link(host, credential, &authenticated.verified, tail, &info)?;
        let submission_error = self.post_generic_link(host, credential, &link).err();
        match self.confirm_committed_user_settings_link(host, credential, &link, &info, &stored) {
            Ok(()) => Ok(()),
            Err(confirm_error) => Err(submission_error.unwrap_or(confirm_error)),
        }
    }

    pub(crate) fn bootstrap_user_settings_from_signup_yubi(
        &self,
        host: &PinnedHost,
        credential: &YubiCredential<'_>,
        expected: &PassphraseUpdateArgument,
    ) -> Result<()> {
        let stored = self.fetch_ppe_parcel_yubi(host, credential)?;
        validate_committed_update(&stored, expected)?;
        self.trust_passphrase_parcel(host, &credential.uid, &stored)?;
        let authenticated = self.authenticate_yubi_and_pin(host, credential)?;
        let (tail, current, _) =
            self.authenticated_user_settings_yubi(host, credential, &authenticated.verified)?;
        let info = PassphraseInfo {
            generation: expected.generation,
            salt: Some(expected.salt),
            stretch_version: expected.stretch_version.protocol_value(),
        };
        if let Some(current) = current {
            return if current == info {
                Ok(())
            } else {
                Err(Error::CredentialBinding(
                    "signup passphrase conflicts with authenticated user settings",
                ))
            };
        }
        let link = self.make_user_settings_link_yubi(
            host,
            credential,
            &authenticated.verified,
            tail,
            &info,
        )?;
        let submission_error = self
            .call_void_with_material(
                host,
                &host.user,
                &foks_rpc::encode_post_generic_link_request(&link)?,
                &credential.subkey_seed,
                &credential.certificate_chain,
            )
            .err();
        let mut expected_with_link = expected.clone();
        expected_with_link.user_settings_link = Some(link);
        match self.confirm_committed_user_settings_yubi(
            host,
            credential,
            &expected_with_link,
            &stored,
        ) {
            Ok(()) => Ok(()),
            Err(confirm_error) => Err(submission_error.unwrap_or(confirm_error)),
        }
    }

    fn verify_passphrase_against_parcel(
        &self,
        host: &PinnedHost,
        uid: &foks_proto::EntityId,
        passphrase: &Passphrase,
        parcel: &PpeParcel,
    ) -> Result<PassphraseVerification> {
        let public_stretch = self.registration_stretch_version(host)?;
        if public_stretch != parcel.stretch_version {
            return Err(Error::CredentialBinding(
                "registration and stored passphrase stretch versions differ",
            ));
        }
        let login = prepare_passphrase_login(passphrase, &parcel.salt, parcel.stretch_version)?;
        let challenge_bytes = self.call_after_vhost_selection(
            host,
            &host.registration,
            &encode_registration_select_vhost_request(host.host_id())?,
            &encode_get_login_challenge_request(uid)?,
        )?;
        let challenge = RegistrationChallenge::decode(&challenge_bytes)?;
        if challenge.payload.entity != *uid || challenge.payload.host != *host.host_id() {
            return Err(Error::CredentialBinding(
                "passphrase challenge identity changed",
            ));
        }
        let signature = login.sign_challenge(&challenge)?;
        let login_bytes = self.call_after_vhost_selection(
            host,
            &host.registration,
            &encode_registration_select_vhost_request(host.host_id())?,
            &encode_passphrase_login_request(uid, &challenge, &signature)?,
        )?;
        let result = PassphraseLoginResult::decode(&login_bytes)?;
        let keyring = login.unlock(uid, host.host_id(), &result)?;
        if keyring.generation() != parcel.generation || result.generation != parcel.generation {
            return Err(Error::CredentialBinding(
                "passphrase login returned a stale PPE generation",
            ));
        }
        Ok(PassphraseVerification {
            generation: keyring.generation(),
        })
    }

    pub fn passphrase_metadata(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
    ) -> Result<PassphraseMetadata> {
        self.fetch_ppe_parcel(host, credential)
            .map(|parcel| metadata(&parcel))
    }

    /// Reboxes an existing PPE parcel to the authenticated current owner PUK
    /// when another device advanced that PUK without carrying the annex.
    pub fn refresh_passphrase_for_current_puk(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
        _authenticated: &crate::AuthenticatedUserOutcome,
    ) -> Result<bool> {
        self.retry_chain_load(host, |current_host| {
            let authenticated = self.authenticate_and_pin(current_host, credential)?;
            self.refresh_passphrase_for_current_puk_at_head(
                current_host,
                credential,
                &authenticated,
            )
        })
    }

    fn refresh_passphrase_for_current_puk_at_head(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
        authenticated: &crate::AuthenticatedUserOutcome,
    ) -> Result<bool> {
        let (_, settings, _) =
            self.authenticated_user_settings(host, credential, &authenticated.verified)?;
        let parcel = match self.fetch_ppe_parcel(host, credential) {
            Ok(parcel) => parcel,
            Err(Error::Rpc(foks_rpc::Error::RemoteStatus {
                code: foks_rpc::STATUS_PASSPHRASE_NOT_FOUND_ERROR,
                ..
            })) if settings.is_none() => return Ok(false),
            Err(Error::Rpc(foks_rpc::Error::RemoteStatus {
                code: foks_rpc::STATUS_PASSPHRASE_NOT_FOUND_ERROR,
                ..
            })) => {
                return Err(Error::CredentialBinding(
                    "server omitted PPE state committed by user settings",
                ));
            }
            Err(error) => return Err(error),
        };
        let owner = current_owner_puk(authenticated)?;
        let boxed_generation = parcel
            .puk_box
            .as_ref()
            .ok_or(Error::CredentialBinding(
                "passphrase has no owner-PUK recovery box",
            ))?
            .puk_generation;
        let settings = settings.ok_or(Error::CredentialBinding(
            "passphrase verification required before background refresh of legacy settings",
        ))?;
        if settings.generation != parcel.generation
            || settings.salt != Some(parcel.salt)
            || settings.stretch_version != parcel.stretch_version.protocol_value()
        {
            return Err(Error::CredentialBinding(
                "authenticated settings and passphrase parcel diverge",
            ));
        }
        // A different legitimate owner device may have changed the parcel
        // since this client last pinned it. Re-anchor only when the current,
        // non-stale owner PUK opens the exact authenticated-settings parcel;
        // historical owner material never satisfies this gate.
        self.trust_passphrase_parcel_if_safe(host, &credential.uid, authenticated, &parcel)?;
        self.require_trusted_passphrase_parcel(host, &credential.uid, &parcel)?;
        if boxed_generation == owner.generation {
            foks_crypto::verify_passphrase_puk_recovery(
                &credential.uid,
                host.host_id(),
                &parcel,
                &owner.seed,
            )?;
            return Ok(false);
        }
        if boxed_generation > owner.generation {
            return Err(Error::CredentialBinding(
                "passphrase owner-PUK generation is ahead of the user chain",
            ));
        }
        let previous = authenticated
            .puks
            .iter()
            .find(|key| key.role == foks_proto::Role::OWNER && key.generation == boxed_generation)
            .ok_or(Error::KeyBinding(
                "historical owner PUK for passphrase rebox is unavailable",
            ))?;
        let update = foks_crypto::rotate_passphrase_for_puk(
            &credential.uid,
            host.host_id(),
            &parcel,
            &previous.seed,
            &owner.seed,
            owner.generation,
        )?;
        let argument = self.with_user_settings_link(
            host,
            credential,
            authenticated,
            update.argument(),
            Some(&parcel),
            PassphrasePukTarget::CurrentOwner,
        )?;
        self.submit_passphrase_update(
            host,
            credential,
            encode_change_passphrase_request(&argument)?,
            &argument,
        )?;
        Ok(true)
    }

    /// Hardware-backed counterpart of [`Self::refresh_passphrase_for_current_puk`].
    /// The caller must keep the YubiKey session and PIN material live only for
    /// this operation; no hardware authorization is persisted for background
    /// use.
    pub fn refresh_passphrase_for_current_puk_yubi(
        &self,
        host: &PinnedHost,
        credential: &YubiCredential<'_>,
        _authenticated: &crate::AuthenticatedUserOutcome,
    ) -> Result<bool> {
        self.retry_chain_load(host, |current_host| {
            let authenticated = self.authenticate_yubi_and_pin(current_host, credential)?;
            self.refresh_passphrase_for_current_puk_yubi_at_head(
                current_host,
                credential,
                &authenticated,
            )
        })
    }

    fn refresh_passphrase_for_current_puk_yubi_at_head(
        &self,
        host: &PinnedHost,
        credential: &YubiCredential<'_>,
        authenticated: &crate::AuthenticatedUserOutcome,
    ) -> Result<bool> {
        let (_, settings, _) =
            self.authenticated_user_settings_yubi(host, credential, &authenticated.verified)?;
        let parcel = match self.fetch_ppe_parcel_yubi(host, credential) {
            Ok(parcel) => parcel,
            Err(Error::Rpc(foks_rpc::Error::RemoteStatus {
                code: foks_rpc::STATUS_PASSPHRASE_NOT_FOUND_ERROR,
                ..
            })) if settings.is_none() => return Ok(false),
            Err(Error::Rpc(foks_rpc::Error::RemoteStatus {
                code: foks_rpc::STATUS_PASSPHRASE_NOT_FOUND_ERROR,
                ..
            })) => {
                return Err(Error::CredentialBinding(
                    "server omitted PPE state committed by user settings",
                ));
            }
            Err(error) => return Err(error),
        };
        let owner = current_owner_puk(authenticated)?;
        let boxed_generation = parcel
            .puk_box
            .as_ref()
            .ok_or(Error::CredentialBinding(
                "passphrase has no owner-PUK recovery box",
            ))?
            .puk_generation;
        let settings = settings.ok_or(Error::CredentialBinding(
            "passphrase verification required before background refresh of legacy settings",
        ))?;
        if settings.generation != parcel.generation
            || settings.salt != Some(parcel.salt)
            || settings.stretch_version != parcel.stretch_version.protocol_value()
        {
            return Err(Error::CredentialBinding(
                "authenticated settings and passphrase parcel diverge",
            ));
        }
        self.trust_passphrase_parcel_if_safe(host, &credential.uid, authenticated, &parcel)?;
        self.require_trusted_passphrase_parcel(host, &credential.uid, &parcel)?;
        if boxed_generation == owner.generation {
            foks_crypto::verify_passphrase_puk_recovery(
                &credential.uid,
                host.host_id(),
                &parcel,
                &owner.seed,
            )?;
            return Ok(false);
        }
        if boxed_generation > owner.generation {
            return Err(Error::CredentialBinding(
                "passphrase owner-PUK generation is ahead of the user chain",
            ));
        }
        let previous = authenticated
            .puks
            .iter()
            .find(|key| key.role == foks_proto::Role::OWNER && key.generation == boxed_generation)
            .ok_or(Error::KeyBinding(
                "historical owner PUK for passphrase rebox is unavailable",
            ))?;
        let update = foks_crypto::rotate_passphrase_for_puk(
            &credential.uid,
            host.host_id(),
            &parcel,
            &previous.seed,
            &owner.seed,
            owner.generation,
        )?;
        let argument = self.with_user_settings_link_yubi(
            host,
            credential,
            authenticated,
            update.argument(),
            Some(&parcel),
            PassphrasePukTarget::CurrentOwner,
        )?;
        self.submit_passphrase_update_yubi(
            host,
            credential,
            encode_change_passphrase_request(&argument)?,
            &argument,
        )?;
        Ok(true)
    }

    pub fn passphrase_metadata_yubi(
        &self,
        host: &PinnedHost,
        credential: &YubiCredential<'_>,
    ) -> Result<PassphraseMetadata> {
        self.fetch_ppe_parcel_yubi(host, credential)
            .map(|parcel| metadata(&parcel))
    }

    /// Reports whether the account has a passphrase annex while preserving
    /// every other transport or authorization failure.
    pub fn passphrase_is_configured(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
    ) -> Result<bool> {
        match self.passphrase_metadata(host, credential) {
            Ok(_) => Ok(true),
            Err(Error::Rpc(foks_rpc::Error::RemoteStatus {
                code: foks_rpc::STATUS_PASSPHRASE_NOT_FOUND_ERROR,
                ..
            })) => Ok(false),
            Err(error) => Err(error),
        }
    }

    pub fn passphrase_is_configured_yubi(
        &self,
        host: &PinnedHost,
        credential: &YubiCredential<'_>,
    ) -> Result<bool> {
        match self.passphrase_metadata_yubi(host, credential) {
            Ok(_) => Ok(true),
            Err(Error::Rpc(foks_rpc::Error::RemoteStatus {
                code: foks_rpc::STATUS_PASSPHRASE_NOT_FOUND_ERROR,
                ..
            })) => Ok(false),
            Err(error) => Err(error),
        }
    }

    pub fn passphrase_salt(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
    ) -> Result<[u8; 16]> {
        let response = self.call(
            host,
            &host.user,
            &foks_rpc::encode_get_passphrase_salt_request()?,
            Some(credential),
        )?;
        Ok(foks_proto::decode_salt(&response)?)
    }

    pub(crate) fn fetch_ppe_parcel(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
    ) -> Result<PpeParcel> {
        let response = self.call(
            host,
            &host.user,
            &encode_get_ppe_parcel_request()?,
            Some(credential),
        )?;
        Ok(PpeParcel::decode(&response)?)
    }

    pub(crate) fn with_user_settings_link(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
        _authenticated: &crate::AuthenticatedUserOutcome,
        argument: PassphraseUpdateArgument,
        predecessor: Option<&PpeParcel>,
        puk_target: PassphrasePukTarget<'_>,
    ) -> Result<PassphraseUpdateArgument> {
        self.retry_chain_load(host, |current_host| {
            let authenticated = self.authenticate_and_pin(current_host, credential)?;
            let (tail, current, _) = self.authenticated_user_settings(
                current_host,
                credential,
                &authenticated.verified,
            )?;
            match (current.as_ref(), predecessor) {
                (None, None) if argument.generation == 1 => {}
                (None, Some(predecessor)) => {
                    let parcel = self.fetch_ppe_parcel(current_host, credential)?;
                    if &parcel != predecessor {
                        return Err(Error::CredentialBinding(
                            "passphrase parcel changed while preparing its update",
                        ));
                    }
                    let owner = current_owner_puk(&authenticated)?;
                    foks_crypto::verify_passphrase_puk_recovery(
                        &credential.uid,
                        current_host.host_id(),
                        &parcel,
                        &owner.seed,
                    )?;
                    self.trust_passphrase_parcel_if_safe(
                        current_host,
                        &credential.uid,
                        &authenticated,
                        &parcel,
                    )?;
                    self.require_trusted_passphrase_parcel(current_host, &credential.uid, &parcel)?;
                    require_next_passphrase_generation(&argument, predecessor)?;
                }
                (Some(current), Some(predecessor)) => {
                    let parcel = self.fetch_ppe_parcel(current_host, credential)?;
                    require_settings_match_parcel(Some(current), &parcel)?;
                    if &parcel != predecessor {
                        return Err(Error::CredentialBinding(
                            "passphrase parcel changed while preparing its update",
                        ));
                    }
                    self.trust_passphrase_parcel_if_safe(
                        current_host,
                        &credential.uid,
                        &authenticated,
                        &parcel,
                    )?;
                    self.require_trusted_passphrase_parcel(current_host, &credential.uid, &parcel)?;
                    require_next_passphrase_generation(&argument, predecessor)?;
                }
                _ => {
                    return Err(Error::CredentialBinding(
                        "passphrase update has no authenticated predecessor",
                    ));
                }
            }
            require_passphrase_puk_target(
                current_host,
                &credential.uid,
                &authenticated,
                &argument,
                puk_target,
            )?;
            let info = PassphraseInfo {
                generation: argument.generation,
                salt: Some(argument.salt),
                stretch_version: argument.stretch_version.protocol_value(),
            };
            let mut result = argument.clone();
            result.user_settings_link = Some(self.make_user_settings_link(
                current_host,
                credential,
                &authenticated.verified,
                tail,
                &info,
            )?);
            Ok(result)
        })
    }

    pub(crate) fn with_user_settings_link_yubi(
        &self,
        host: &PinnedHost,
        credential: &YubiCredential<'_>,
        _authenticated: &crate::AuthenticatedUserOutcome,
        argument: PassphraseUpdateArgument,
        predecessor: Option<&PpeParcel>,
        puk_target: PassphrasePukTarget<'_>,
    ) -> Result<PassphraseUpdateArgument> {
        self.retry_chain_load(host, |current_host| {
            let authenticated = self.authenticate_yubi_and_pin(current_host, credential)?;
            let (tail, current, _) = self.authenticated_user_settings_yubi(
                current_host,
                credential,
                &authenticated.verified,
            )?;
            match (current.as_ref(), predecessor) {
                (None, None) if argument.generation == 1 => {}
                (None, Some(predecessor)) => {
                    let parcel = self.fetch_ppe_parcel_yubi(current_host, credential)?;
                    if &parcel != predecessor {
                        return Err(Error::CredentialBinding(
                            "passphrase parcel changed while preparing its update",
                        ));
                    }
                    let owner = current_owner_puk(&authenticated)?;
                    foks_crypto::verify_passphrase_puk_recovery(
                        &credential.uid,
                        current_host.host_id(),
                        &parcel,
                        &owner.seed,
                    )?;
                    self.trust_passphrase_parcel_if_safe(
                        current_host,
                        &credential.uid,
                        &authenticated,
                        &parcel,
                    )?;
                    self.require_trusted_passphrase_parcel(current_host, &credential.uid, &parcel)?;
                    require_next_passphrase_generation(&argument, predecessor)?;
                }
                (Some(current), Some(predecessor)) => {
                    let parcel = self.fetch_ppe_parcel_yubi(current_host, credential)?;
                    require_settings_match_parcel(Some(current), &parcel)?;
                    if &parcel != predecessor {
                        return Err(Error::CredentialBinding(
                            "passphrase parcel changed while preparing its update",
                        ));
                    }
                    self.trust_passphrase_parcel_if_safe(
                        current_host,
                        &credential.uid,
                        &authenticated,
                        &parcel,
                    )?;
                    self.require_trusted_passphrase_parcel(current_host, &credential.uid, &parcel)?;
                    require_next_passphrase_generation(&argument, predecessor)?;
                }
                _ => {
                    return Err(Error::CredentialBinding(
                        "passphrase update has no authenticated predecessor",
                    ));
                }
            }
            require_passphrase_puk_target(
                current_host,
                &credential.uid,
                &authenticated,
                &argument,
                puk_target,
            )?;
            let info = PassphraseInfo {
                generation: argument.generation,
                salt: Some(argument.salt),
                stretch_version: argument.stretch_version.protocol_value(),
            };
            let mut result = argument.clone();
            result.user_settings_link = Some(self.make_user_settings_link_yubi(
                current_host,
                credential,
                &authenticated.verified,
                tail,
                &info,
            )?);
            Ok(result)
        })
    }

    pub(crate) fn fetch_ppe_parcel_yubi(
        &self,
        host: &PinnedHost,
        credential: &YubiCredential<'_>,
    ) -> Result<PpeParcel> {
        let response = self.call_with_material(
            host,
            &host.user,
            &encode_get_ppe_parcel_request()?,
            &credential.subkey_seed,
            &credential.certificate_chain,
        )?;
        Ok(PpeParcel::decode(&response)?)
    }

    fn trust_passphrase_parcel(
        &self,
        host: &PinnedHost,
        uid: &foks_proto::EntityId,
        parcel: &PpeParcel,
    ) -> Result<()> {
        let hash = local_ppe_parcel_hash(parcel)?;
        crate::HardStateStore::open(&host.database_path)?.trust_user_passphrase_parcel(
            host.host_id().as_bytes(),
            uid.as_bytes(),
            &hash,
        )?;
        Ok(())
    }

    fn trust_passphrase_parcel_if_safe(
        &self,
        host: &PinnedHost,
        uid: &foks_proto::EntityId,
        authenticated: &crate::AuthenticatedUserOutcome,
        parcel: &PpeParcel,
    ) -> Result<bool> {
        // A stale role means a revoked credential might still know this PUK;
        // it does not make the authenticated PPE parcel untrustworthy. The
        // exact parcel is bound to the signed settings chain below and must
        // open under the currently published owner PUK. Allowing that proof
        // to refresh the local pin is what lets another owner rotate the stale
        // PUK instead of deadlocking on the pin it is trying to establish.
        let Ok(owner) = current_owner_puk(authenticated) else {
            return Ok(false);
        };
        if parcel
            .puk_box
            .as_ref()
            .is_none_or(|boxed| boxed.puk_generation != owner.generation)
            || foks_crypto::verify_passphrase_puk_recovery(uid, host.host_id(), parcel, &owner.seed)
                .is_err()
        {
            return Ok(false);
        }
        self.trust_passphrase_parcel(host, uid, parcel)?;
        Ok(true)
    }

    pub(crate) fn require_trusted_passphrase_parcel(
        &self,
        host: &PinnedHost,
        uid: &foks_proto::EntityId,
        parcel: &PpeParcel,
    ) -> Result<()> {
        let expected = crate::HardStateStore::open(&host.database_path)?
            .trusted_user_passphrase_parcel_hash(host.host_id().as_bytes(), uid.as_bytes())?;
        if expected != Some(local_ppe_parcel_hash(parcel)?) {
            return Err(Error::AccountRequest(
                "passphrase state needs local verification before unattended reboxing",
            ));
        }
        Ok(())
    }

    fn submit_passphrase_update(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
        request: Vec<u8>,
        expected: &foks_proto::PassphraseUpdateArgument,
    ) -> Result<PpeParcel> {
        let submission_error = match self.call_void(host, &host.user, &request, credential) {
            Ok(()) => None,
            Err(error @ Error::Rpc(foks_rpc::Error::RemoteStatus { .. })) => return Err(error),
            Err(error) => Some(error),
        };
        let stored = match self.fetch_ppe_parcel(host, credential) {
            Ok(stored) => stored,
            Err(read_error) => return Err(submission_error.unwrap_or(read_error)),
        };
        if let Err(binding_error) = validate_committed_or_superseded(&stored, expected) {
            return Err(submission_error.unwrap_or(binding_error));
        }
        self.confirm_committed_user_settings(host, credential, expected, &stored)?;
        Ok(stored)
    }

    fn submit_passphrase_update_yubi(
        &self,
        host: &PinnedHost,
        credential: &YubiCredential<'_>,
        request: Vec<u8>,
        expected: &foks_proto::PassphraseUpdateArgument,
    ) -> Result<PpeParcel> {
        let submission_error = match self.call_void_with_material(
            host,
            &host.user,
            &request,
            &credential.subkey_seed,
            &credential.certificate_chain,
        ) {
            Ok(()) => None,
            Err(error @ Error::Rpc(foks_rpc::Error::RemoteStatus { .. })) => return Err(error),
            Err(error) => Some(error),
        };
        let stored = match self.fetch_ppe_parcel_yubi(host, credential) {
            Ok(stored) => stored,
            Err(read_error) => return Err(submission_error.unwrap_or(read_error)),
        };
        if let Err(binding_error) = validate_committed_or_superseded(&stored, expected) {
            return Err(submission_error.unwrap_or(binding_error));
        }
        self.confirm_committed_user_settings_yubi(host, credential, expected, &stored)?;
        Ok(stored)
    }

    fn authenticated_stretch_version(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
    ) -> Result<StretchVersion> {
        let response = self.call(
            host,
            &host.user,
            &encode_user_stretch_version_request()?,
            Some(credential),
        )?;
        let version = decode_stretch_version(&response)?;
        require_production_stretch(version)
    }

    pub(crate) fn confirm_committed_user_settings(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
        expected: &PassphraseUpdateArgument,
        stored: &PpeParcel,
    ) -> Result<()> {
        let Some(link) = expected.user_settings_link.as_ref() else {
            return Ok(());
        };
        self.confirm_committed_user_settings_link(
            host,
            credential,
            link,
            &settings_info_from_link(link)?,
            stored,
        )?;
        if validate_committed_update(stored, expected).is_ok() {
            self.trust_passphrase_parcel(host, &credential.uid, stored)?;
        }
        Ok(())
    }

    fn confirm_committed_user_settings_link(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
        expected_link: &PostGenericLinkArgument,
        expected_info: &PassphraseInfo,
        stored: &PpeParcel,
    ) -> Result<()> {
        let expected_hash = foks_crypto::prefixed_hash_signable(
            LINK_OUTER_TYPE_ID,
            &expected_link.link.encoded()?,
        )?;
        self.retry_user_settings_publication(|| {
            self.retry_chain_load(host, |current_host| {
                let authenticated = self.authenticate_and_pin(current_host, credential)?;
                let (_, current, history) = self.authenticated_user_settings(
                    current_host,
                    credential,
                    &authenticated.verified,
                )?;
                require_parcel_recoverable_by_current_owner(current_host, &authenticated, stored)?;
                let expected_sequence = expected_link.link.decode_generic()?.sequence;
                if !history.iter().any(|(sequence, hash, info)| {
                    *sequence == expected_sequence
                        && *hash == expected_hash
                        && info == expected_info
                }) || current.as_ref() != Some(&passphrase_info(stored))
                {
                    return Err(Error::CredentialBinding(
                        "server omitted or replaced the submitted user-settings link",
                    ));
                }
                Ok(())
            })
        })
    }

    pub(crate) fn confirm_committed_user_settings_yubi(
        &self,
        host: &PinnedHost,
        credential: &YubiCredential<'_>,
        expected: &PassphraseUpdateArgument,
        stored: &PpeParcel,
    ) -> Result<()> {
        let Some(expected_link) = expected.user_settings_link.as_ref() else {
            return Ok(());
        };
        let expected_hash = foks_crypto::prefixed_hash_signable(
            LINK_OUTER_TYPE_ID,
            &expected_link.link.encoded()?,
        )?;
        let expected_info = settings_info_from_link(expected_link)?;
        self.retry_user_settings_publication(|| {
            self.retry_chain_load(host, |current_host| {
                let authenticated = self.authenticate_yubi_and_pin(current_host, credential)?;
                let (_, current, history) = self.authenticated_user_settings_yubi(
                    current_host,
                    credential,
                    &authenticated.verified,
                )?;
                require_parcel_recoverable_by_current_owner(current_host, &authenticated, stored)?;
                let expected_sequence = expected_link.link.decode_generic()?.sequence;
                if !history.iter().any(|(sequence, hash, info)| {
                    *sequence == expected_sequence
                        && *hash == expected_hash
                        && info == &expected_info
                }) || current.as_ref() != Some(&passphrase_info(stored))
                {
                    return Err(Error::CredentialBinding(
                        "server omitted or replaced the submitted user-settings link",
                    ));
                }
                Ok(())
            })
        })?;
        if validate_committed_update(stored, expected).is_ok() {
            self.trust_passphrase_parcel(host, &credential.uid, stored)?;
        }
        Ok(())
    }

    fn retry_user_settings_publication<T>(
        &self,
        mut operation: impl FnMut() -> Result<T>,
    ) -> Result<T> {
        const ATTEMPTS: usize = 11;
        for attempt in 0..ATTEMPTS {
            match operation() {
                Ok(value) => return Ok(value),
                Err(error)
                    if attempt + 1 < ATTEMPTS
                        && (matches!(
                            error,
                            Error::Verify(foks_verify::Error::UserMerkleProof)
                                | Error::CredentialBinding(
                                    "server omitted or replaced the submitted user-settings link"
                                )
                        ) || crate::auth::retryable_chain_load_error(&error)) =>
                {
                    std::thread::sleep(std::time::Duration::from_millis(25_u64 << attempt));
                }
                Err(error) => return Err(error),
            }
        }
        unreachable!("bounded publication retry always returns")
    }

    fn authenticated_stretch_version_yubi(
        &self,
        host: &PinnedHost,
        credential: &YubiCredential<'_>,
    ) -> Result<StretchVersion> {
        let response = self.call_with_material(
            host,
            &host.user,
            &encode_user_stretch_version_request()?,
            &credential.subkey_seed,
            &credential.certificate_chain,
        )?;
        let version = decode_stretch_version(&response)?;
        require_production_stretch(version)
    }

    fn registration_stretch_version(&self, host: &PinnedHost) -> Result<StretchVersion> {
        let response = self.call_after_vhost_selection(
            host,
            &host.registration,
            &encode_registration_select_vhost_request(host.host_id())?,
            &encode_registration_stretch_version_request()?,
        )?;
        require_production_stretch(decode_stretch_version(&response)?)
    }
}

fn require_production_stretch(version: StretchVersion) -> Result<StretchVersion> {
    if version == StretchVersion::V1 {
        Ok(version)
    } else {
        Err(Error::CredentialBinding(
            "server selected a test-only passphrase stretch version",
        ))
    }
}

fn metadata(parcel: &PpeParcel) -> PassphraseMetadata {
    PassphraseMetadata {
        salt: parcel.salt,
        generation: parcel.generation,
        stretch_version: parcel.stretch_version,
    }
}

fn passphrase_info(parcel: &PpeParcel) -> PassphraseInfo {
    PassphraseInfo {
        generation: parcel.generation,
        salt: Some(parcel.salt),
        stretch_version: parcel.stretch_version.protocol_value(),
    }
}

fn passphrase_argument(parcel: &PpeParcel) -> PassphraseUpdateArgument {
    PassphraseUpdateArgument {
        verify_key: parcel.verify_key.clone(),
        salt: parcel.salt,
        generation: parcel.generation,
        skmwk_box: parcel.skmwk_box.clone(),
        passphrase_box: parcel.passphrase_box.clone(),
        puk_box: parcel.puk_box.clone(),
        stretch_version: parcel.stretch_version,
        user_settings_link: None,
    }
}

fn local_ppe_parcel_hash(parcel: &PpeParcel) -> Result<[u8; 32]> {
    Ok(foks_crypto::prefixed_hash(
        LOCAL_PPE_PARCEL_HASH_TYPE_ID,
        &parcel.encoded()?,
    ))
}

fn parcel_from_argument(argument: &PassphraseUpdateArgument) -> PpeParcel {
    PpeParcel {
        skmwk_box: argument.skmwk_box.clone(),
        generation: argument.generation,
        passphrase_box: argument.passphrase_box.clone(),
        puk_box: argument.puk_box.clone(),
        salt: argument.salt,
        stretch_version: argument.stretch_version,
        verify_key: argument.verify_key.clone(),
    }
}

fn require_passphrase_puk_target(
    host: &PinnedHost,
    uid: &foks_proto::EntityId,
    authenticated: &crate::AuthenticatedUserOutcome,
    argument: &PassphraseUpdateArgument,
    target: PassphrasePukTarget<'_>,
) -> Result<()> {
    let proposed = parcel_from_argument(argument);
    let proposed_generation = proposed
        .puk_box
        .as_ref()
        .ok_or(Error::CredentialBinding(
            "passphrase update omitted owner-PUK recovery",
        ))?
        .puk_generation;
    let recovery_seed = match target {
        PassphrasePukTarget::CurrentOwner => {
            let owner = current_owner_puk(authenticated)?;
            if proposed_generation != owner.generation {
                return Err(Error::CredentialBinding(
                    "passphrase update targets a stale owner PUK",
                ));
            }
            &owner.seed
        }
        PassphrasePukTarget::FutureOwner {
            previous_seed,
            new_seed,
            new_generation,
        } => {
            let owner = current_owner_puk(authenticated)?;
            if owner.seed != *previous_seed
                || owner.generation.checked_add(1) != Some(new_generation)
                || proposed_generation != new_generation
            {
                return Err(Error::CredentialBinding(
                    "passphrase annex is not bound to the pending owner-PUK rotation",
                ));
            }
            new_seed
        }
    };
    foks_crypto::verify_passphrase_puk_recovery(uid, host.host_id(), &proposed, recovery_seed)?;
    Ok(())
}

fn require_parcel_recoverable_by_current_owner(
    host: &PinnedHost,
    authenticated: &crate::AuthenticatedUserOutcome,
    parcel: &PpeParcel,
) -> Result<()> {
    let owner = current_owner_puk(authenticated)?;
    if parcel
        .puk_box
        .as_ref()
        .is_none_or(|boxed| boxed.puk_generation != owner.generation)
    {
        return Err(Error::CredentialBinding(
            "committed passphrase parcel targets a stale owner PUK",
        ));
    }
    foks_crypto::verify_passphrase_puk_recovery(
        authenticated.verified.uid(),
        host.host_id(),
        parcel,
        &owner.seed,
    )?;
    Ok(())
}

fn settings_info_from_link(link: &PostGenericLinkArgument) -> Result<PassphraseInfo> {
    match link.link.decode_generic()?.payload {
        GenericLinkPayload::UserSettings(info) => Ok(info),
        GenericLinkPayload::TeamMembership(_) => Err(Error::CredentialBinding(
            "passphrase update carries a non-settings generic link",
        )),
    }
}

fn require_settings_match_parcel(
    settings: Option<&PassphraseInfo>,
    parcel: &PpeParcel,
) -> Result<()> {
    if settings.is_some_and(|settings| settings != &passphrase_info(parcel)) {
        return Err(Error::CredentialBinding(
            "authenticated settings and passphrase parcel diverge",
        ));
    }
    Ok(())
}

fn require_next_passphrase_generation(
    argument: &PassphraseUpdateArgument,
    predecessor: &PpeParcel,
) -> Result<()> {
    if argument.generation
        != predecessor
            .generation
            .checked_add(1)
            .ok_or(Error::CredentialBinding("passphrase generation overflow"))?
    {
        return Err(Error::CredentialBinding(
            "passphrase update did not advance the authenticated generation",
        ));
    }
    Ok(())
}

pub(crate) fn validate_committed_update(
    stored: &PpeParcel,
    expected: &foks_proto::PassphraseUpdateArgument,
) -> Result<()> {
    if stored.verify_key != expected.verify_key
        || stored.salt != expected.salt
        || stored.generation != expected.generation
        || stored.stretch_version != expected.stretch_version
        || stored.skmwk_box != expected.skmwk_box
        || stored.passphrase_box != expected.passphrase_box
        || stored.puk_box != expected.puk_box
    {
        return Err(Error::CredentialBinding(
            "server did not retain the submitted passphrase boxes",
        ));
    }
    Ok(())
}

pub(crate) fn validate_committed_or_superseded(
    stored: &PpeParcel,
    expected: &foks_proto::PassphraseUpdateArgument,
) -> Result<()> {
    match validate_committed_update(stored, expected) {
        Ok(()) => Ok(()),
        Err(_) if stored.generation > expected.generation => Ok(()),
        Err(error) => Err(error),
    }
}
