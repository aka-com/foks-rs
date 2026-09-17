//! Passphrase enrollment, rotation, and public challenge verification.

pub use foks_crypto::Passphrase;
use foks_crypto::{
    change_passphrase_with_puk, create_passphrase_enrollment, prepare_passphrase_login,
};
use foks_proto::{
    decode_stretch_version, PassphraseLoginResult, PpeParcel, RegistrationChallenge, StretchVersion,
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

impl FoksClient {
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
        let argument = update.argument();
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
        let argument = update.argument();
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
        let argument = update.argument();
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
        let argument = update.argument();
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
        let parcel = self.fetch_ppe_parcel(host, credential)?;
        self.verify_passphrase_against_parcel(host, &credential.uid, passphrase, &parcel)
    }

    pub fn verify_passphrase_yubi(
        &self,
        host: &PinnedHost,
        credential: &YubiCredential<'_>,
        passphrase: &Passphrase,
    ) -> Result<PassphraseVerification> {
        let parcel = self.fetch_ppe_parcel_yubi(host, credential)?;
        self.verify_passphrase_against_parcel(host, &credential.uid, passphrase, &parcel)
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
        if let Err(binding_error) = validate_committed_update(&stored, expected) {
            return Err(submission_error.unwrap_or(binding_error));
        }
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
        if let Err(binding_error) = validate_committed_update(&stored, expected) {
            return Err(submission_error.unwrap_or(binding_error));
        }
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
