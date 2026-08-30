//! Backup-key enrollment and recovery-device provisioning.

use foks_crypto::{
    derive_device_public, derive_shared_verify_key, make_backup_provision_link,
    make_software_provision_link_from_backup_credential, open_puk_parcel_with_for_role,
    open_puk_seed_chain, seal_backup_puk_boxes_from_credential, seal_software_puk_boxes, BackupKey,
    BackupKeyMaterial, HybridSecretDecapsulator, PukBoxRandomness, SoftwareProvisionInput,
    SoftwarePukBoxInput, UserMutationBase,
};
use foks_proto::{
    DeviceLabel, DeviceLabelNameAndCommitmentKey, DeviceType, EntityId, LookupUserResult,
    ProvisionDeviceArgument, PukParcel, RegistrationChallenge, Role, ENTITY_BACKUP_KEY,
    ENTITY_PUK_VERIFY,
};
use foks_rpc::{
    encode_get_client_cert_chain_request_at, encode_get_puk_for_role_request,
    encode_get_uid_lookup_challenge_request, encode_load_user_chain_request,
    encode_lookup_uid_by_device_request, encode_provision_device_request,
    encode_registration_select_vhost_request,
};
use foks_snowpack::{decode, Value};
use foks_verify::{normalize_device_name, verify_user_chain, VerifiedDevice};

use crate::{
    fix_device_name, now_milliseconds, random_bytes, AuthenticatedUserOutcome, DeviceCredential,
    Error, FoksClient, HardStateStore, NewSoftwareDeviceSecrets, PinnedHost,
    ProvisionedSoftwareDevice, Result, SoftwareDeviceProvisionRequest, UserPrivateKey,
};

const CHALLENGE_WINDOW_MILLISECONDS: u64 = 4 * 60 * 60 * 1_000;

/// A backup key located through an untrusted registration response.
///
/// The HESP seed is consumed during lookup. This value retains only derived
/// zeroizing key material and should not be persisted. Its claimed user and
/// role are not authoritative until `authenticate_backup_and_pin` succeeds.
pub struct LocatedBackupCredential {
    uid: EntityId,
    role: Role,
    key: BackupKeyMaterial,
    certificate_chain: Vec<Vec<u8>>,
}

/// Authenticated user state after a backup key is observed in the chain.
pub struct BackupEnrollmentOutcome {
    pub authenticated: AuthenticatedUserOutcome,
}

impl FoksClient {
    /// Enrolls a caller-generated backup key at an existing PUK role.
    ///
    /// The caller must durably record `backup.phrase()` before invoking this
    /// method. FOKS intentionally does not persist the phrase or seed locally.
    /// Repeating the call with the same authenticated key binding and role
    /// reconciles a previously committed enrollment.
    pub fn enroll_backup_key(
        &self,
        host: &PinnedHost,
        existing: &DeviceCredential,
        role: Role,
        backup: &BackupKey,
    ) -> Result<BackupEnrollmentOutcome> {
        if role == Role::NONE {
            return Err(Error::AccountRequest("backup key requires a PUK role"));
        }
        let authenticated = self.authenticate_and_pin(host, existing)?;
        let signer = derive_device_public(&existing.seed)?;
        let enrolled_signer = authenticated
            .verified
            .devices()
            .iter()
            .find(|device| device.id == signer.id)
            .ok_or(Error::UserBinding("signing device is not enrolled"))?;
        if enrolled_signer.role != Role::OWNER || role > enrolled_signer.role {
            return Err(Error::AccountRequest(
                "backup enrollment requires an owner and a reachable role",
            ));
        }
        let backup_public = backup.public_material()?;
        if credential_already_enrolled(authenticated.verified.devices(), &backup_public, role)? {
            return Ok(BackupEnrollmentOutcome { authenticated });
        }
        let role_key = authenticated
            .verified
            .shared_key(role)
            .ok_or(Error::AccountRequest("backup role has no existing PUK"))?;
        let role_puks = if role == enrolled_signer.role {
            None
        } else {
            Some(self.load_puks_for_role(host, existing, &authenticated.verified, role)?)
        };
        let private = role_puks
            .as_deref()
            .unwrap_or(&authenticated.puks)
            .iter()
            .find(|key| {
                key.role == role
                    && key.generation == role_key.generation
                    && derive_shared_verify_key(&key.seed, ENTITY_PUK_VERIFY)
                        .is_ok_and(|verify| verify == role_key.verify_key)
            })
            .ok_or(Error::KeyBinding("current backup-role PUK is not loaded"))?;

        let display_name = backup.device_name();
        let normalized_name = normalize_device_name(display_name.as_bytes())
            .ok_or(Error::AccountRequest("invalid backup-key device name"))?;
        let device_name = DeviceLabelNameAndCommitmentKey {
            label: DeviceLabel {
                device_type: DeviceType::Backup,
                normalized_name,
                serial: 1,
            },
            normalization_version: 0,
            display_name: display_name.into_bytes(),
            commitment_key: random_bytes()?,
        };
        let next_tree_location = random_bytes()?;
        let material = make_backup_provision_link(
            &SoftwareProvisionInput {
                base: UserMutationBase {
                    uid: authenticated.verified.uid(),
                    host: authenticated.verified.host(),
                    seqno: authenticated
                        .verified
                        .chain_seqno()
                        .checked_add(1)
                        .ok_or(Error::AccountRequest("user sequence overflow"))?,
                    previous: authenticated.verified.chain_tail_hash(),
                    root: &authenticated.verified.tree_root(),
                    time: now_milliseconds()?,
                    next_tree_location,
                },
                role,
                device_label: &device_name.label,
                device_name_commitment_key: device_name.commitment_key,
            },
            &existing.seed,
            backup,
            None,
        )?;
        let box_inputs = [SoftwarePukBoxInput {
            seed: &private.seed,
            generation: private.generation,
            role,
            receiver: &backup_public,
        }];
        let randomness = [PukBoxRandomness {
            kem_message: random_bytes()?,
            nonce: random_bytes()?,
        }];
        let puk_boxes = seal_software_puk_boxes(
            host.host_id(),
            &existing.seed,
            random_bytes()?,
            &box_inputs,
            &randomness,
        )?;
        let encoded = encode_provision_device_request(&ProvisionDeviceArgument {
            link: &material.link,
            puk_boxes: &puk_boxes,
            device_name: &device_name,
            next_tree_location,
            self_token: random_bytes()?,
            hepks: &[backup_public.hepk],
            subkey_box: None,
            yubi_pq_hint: None,
        })?;
        let post_error = self.call_void(host, &host.user, &encoded, existing).err();
        let authenticated = match self.wait_for_user_transition(host, existing, |user| {
            user.devices()
                .iter()
                .any(|device| device.id == backup_public.id && device.role == role)
        }) {
            Ok(authenticated) => authenticated,
            Err(_) if post_error.is_some() => return Err(post_error.expect("checked above")),
            Err(error) => return Err(error),
        };
        Ok(BackupEnrollmentOutcome { authenticated })
    }

    /// Locates the account controlled by an enrolled backup key and obtains
    /// an ephemeral mTLS credential. No browser or interactive KEX is used.
    pub fn load_backup_key(
        &self,
        host: &PinnedHost,
        backup: BackupKey,
    ) -> Result<LocatedBackupCredential> {
        let key = backup.into_key_material()?;
        let public_id = key.entity_id().clone();
        let challenge_bytes = self.call_after_vhost_selection(
            host,
            &host.registration,
            &encode_registration_select_vhost_request(host.host_id())?,
            &encode_get_uid_lookup_challenge_request(&public_id)?,
        )?;
        let challenge = RegistrationChallenge::decode(&challenge_bytes)?;
        let now = current_milliseconds()?;
        if challenge.payload.entity != public_id
            || challenge.payload.host != *host.host_id()
            || challenge.payload.time.abs_diff(now) > CHALLENGE_WINDOW_MILLISECONDS
        {
            return Err(Error::CredentialBinding(
                "registration lookup challenge is not bound to this backup key and host",
            ));
        }
        let signature = key.sign_registration_challenge(&challenge)?;
        let lookup_bytes = self.call_after_vhost_selection(
            host,
            &host.registration,
            &encode_registration_select_vhost_request(host.host_id())?,
            &encode_lookup_uid_by_device_request(&public_id, &challenge, &signature)?,
        )?;
        let lookup = LookupUserResult::decode(&lookup_bytes)?;
        if lookup.host != *host.host_id()
            || lookup.role == Role::NONE
            || lookup.yubi_pq_hint.is_some()
        {
            return Err(Error::CredentialBinding(
                "backup lookup result is not bound to the selected host and key type",
            ));
        }
        let certificate_chain = self.fetch_key_certificate_chain(host, &lookup.uid, &public_id)?;
        Ok(LocatedBackupCredential {
            uid: lookup.uid,
            role: lookup.role,
            key,
            certificate_chain,
        })
    }

    pub fn authenticate_backup_and_pin(
        &self,
        host: &PinnedHost,
        credential: &LocatedBackupCredential,
    ) -> Result<AuthenticatedUserOutcome> {
        let (merkle_acceptance, merkle) = self.advance_merkle_root(host)?;
        let chain_request = encode_load_user_chain_request(credential.uid.as_bytes(), 1)?;
        let chain_bytes = self.call_with_pkcs8_material(
            host,
            &host.user,
            &chain_request,
            credential.key.expose_signing_key_pkcs8()?,
            &credential.certificate_chain,
        )?;
        let authenticated_roots =
            self.authenticate_user_chain_roots(host, &merkle, &chain_bytes)?;
        let verified = verify_user_chain(
            &chain_bytes,
            &credential.uid,
            host.host_id(),
            &authenticated_roots,
            &merkle.root().hostchain,
        )?;
        let enrolled = verified
            .devices()
            .iter()
            .find(|device| {
                device.id == *credential.key.entity_id()
                    && device.hepk == *credential.key.hepk()
                    && device.id.entity_type() == ENTITY_BACKUP_KEY
            })
            .ok_or(Error::CredentialBinding(
                "backup key is not enrolled in the verified user chain",
            ))?;
        if enrolled.role != credential.role {
            return Err(Error::CredentialBinding("backup-key role changed"));
        }
        let puk_request = encode_get_puk_for_role_request(enrolled.role, enrolled.id.as_bytes())?;
        let parcel_bytes = self.call_with_pkcs8_material(
            host,
            &host.user,
            &puk_request,
            credential.key.expose_signing_key_pkcs8()?,
            &credential.certificate_chain,
        )?;
        let parcel = PukParcel::decode(&parcel_bytes)?;
        let sender = verified
            .devices()
            .iter()
            .find(|device| device.id == parcel.sender)
            .ok_or(Error::UserBinding("PUK parcel sender is not enrolled"))?;
        let role_key = verified
            .shared_key(enrolled.role)
            .ok_or(Error::UserBinding("backup role has no PUK"))?;
        let clear = open_puk_parcel_with_for_role(
            &parcel,
            &credential.key,
            &sender.hepk,
            &role_key.verify_key,
            &role_key.hepk,
            role_key.generation,
            host.host_id(),
            enrolled.role,
        )?;
        let puks = open_puk_seed_chain(clear, &parcel, verified.uid(), host.host_id())?
            .into_iter()
            .map(|key| UserPrivateKey {
                role: key.role,
                generation: key.generation,
                seed: key.into_seed(),
            })
            .collect();
        let mut store = HardStateStore::open(&host.database_path)?;
        let acceptance = store.accept_verified_user(&verified.hard_state_snapshot()?)?;
        Ok(AuthenticatedUserOutcome {
            merkle_acceptance,
            acceptance,
            verified,
            puks,
        })
    }

    /// Uses an ephemeral backup credential to provision a durable software
    /// device, then returns only the new device credential.
    ///
    /// The operation is idempotent for the same device seed and role. This
    /// lets a caller retry after losing the provision response or failing to
    /// fetch the new certificate, provided the seed was made durable before
    /// the first attempt.
    pub fn recover_software_device(
        &self,
        host: &PinnedHost,
        backup: LocatedBackupCredential,
        request: SoftwareDeviceProvisionRequest,
        secrets: NewSoftwareDeviceSecrets,
    ) -> Result<ProvisionedSoftwareDevice> {
        if request.role == Role::NONE || request.serial == 0 {
            return Err(Error::AccountRequest(
                "invalid recovery device role or serial",
            ));
        }
        let authenticated = self.authenticate_backup_and_pin(host, &backup)?;
        if backup.role != Role::OWNER {
            return Err(Error::AccountRequest(
                "replacement-device provisioning requires an owner backup key",
            ));
        }
        if request.role > backup.role {
            return Err(Error::AccountRequest(
                "recovery device role exceeds the backup-key role",
            ));
        }
        let role_key = authenticated
            .verified
            .shared_key(request.role)
            .ok_or(Error::AccountRequest("recovery role has no existing PUK"))?;
        if secrets.introduced_puk_seed.is_some() {
            return Err(Error::AccountRequest(
                "backup recovery cannot introduce a new PUK role",
            ));
        }
        let new_device = derive_device_public(&secrets.device_seed)?;
        if credential_already_enrolled(authenticated.verified.devices(), &new_device, request.role)?
        {
            let certificate_chain =
                self.fetch_device_certificate_chain(host, &backup.uid, &secrets.device_seed)?;
            let credential = DeviceCredential {
                uid: backup.uid,
                seed: secrets.device_seed,
                certificate_chain,
            };
            let authenticated = self.authenticate_and_pin(host, &credential)?;
            return Ok(ProvisionedSoftwareDevice {
                operation_id: None,
                credential,
                authenticated,
            });
        }
        let role_puks = if request.role == backup.role {
            None
        } else {
            Some(self.load_backup_puks_for_role(
                host,
                &backup,
                &authenticated.verified,
                request.role,
            )?)
        };
        let private = role_puks
            .as_deref()
            .unwrap_or(&authenticated.puks)
            .iter()
            .find(|key| key.role == request.role && key.generation == role_key.generation)
            .ok_or(Error::KeyBinding("current recovery-role PUK is not loaded"))?;
        let display_name = fix_device_name(&request.device_name);
        let normalized_name = normalize_device_name(display_name.as_bytes())
            .ok_or(Error::AccountRequest("invalid recovery device name"))?;
        let device_name = DeviceLabelNameAndCommitmentKey {
            label: DeviceLabel {
                device_type: DeviceType::Computer,
                normalized_name,
                serial: request.serial,
            },
            normalization_version: 0,
            display_name: display_name.into_bytes(),
            commitment_key: random_bytes()?,
        };
        let next_tree_location = random_bytes()?;
        let material = make_software_provision_link_from_backup_credential(
            &SoftwareProvisionInput {
                base: UserMutationBase {
                    uid: authenticated.verified.uid(),
                    host: authenticated.verified.host(),
                    seqno: authenticated
                        .verified
                        .chain_seqno()
                        .checked_add(1)
                        .ok_or(Error::AccountRequest("user sequence overflow"))?,
                    previous: authenticated.verified.chain_tail_hash(),
                    root: &authenticated.verified.tree_root(),
                    time: now_milliseconds()?,
                    next_tree_location,
                },
                role: request.role,
                device_label: &device_name.label,
                device_name_commitment_key: device_name.commitment_key,
            },
            &backup.key,
            &secrets.device_seed,
            None,
        )?;
        let inputs = [SoftwarePukBoxInput {
            seed: &private.seed,
            generation: private.generation,
            role: request.role,
            receiver: &new_device,
        }];
        let randomness = [PukBoxRandomness {
            kem_message: random_bytes()?,
            nonce: random_bytes()?,
        }];
        let boxes = seal_backup_puk_boxes_from_credential(
            host.host_id(),
            &backup.key,
            random_bytes()?,
            &inputs,
            &randomness,
        )?;
        let encoded = encode_provision_device_request(&ProvisionDeviceArgument {
            link: &material.link,
            puk_boxes: &boxes,
            device_name: &device_name,
            next_tree_location,
            self_token: *secrets.self_token,
            hepks: &[new_device.hepk],
            subkey_box: None,
            yubi_pq_hint: None,
        })?;
        let post_error = self
            .call_void_with_pkcs8_material(
                host,
                &host.user,
                &encoded,
                backup.key.expose_signing_key_pkcs8()?,
                &backup.certificate_chain,
            )
            .err();
        let certificate_chain =
            match self.fetch_device_certificate_chain(host, &backup.uid, &secrets.device_seed) {
                Ok(chain) => chain,
                Err(_) if post_error.is_some() => return Err(post_error.expect("checked above")),
                Err(error) => return Err(error),
            };
        let credential = DeviceCredential {
            uid: backup.uid,
            seed: secrets.device_seed,
            certificate_chain,
        };
        let authenticated = self.wait_for_user_transition(host, &credential, |user| {
            user.devices()
                .iter()
                .any(|device| device.id == new_device.id && device.role == request.role)
        })?;
        Ok(ProvisionedSoftwareDevice {
            operation_id: None,
            credential,
            authenticated,
        })
    }

    fn fetch_key_certificate_chain(
        &self,
        host: &PinnedHost,
        uid: &EntityId,
        key: &EntityId,
    ) -> Result<Vec<Vec<u8>>> {
        let response = self.call_after_vhost_selection(
            host,
            &host.registration,
            &encode_registration_select_vhost_request(host.host_id())?,
            &encode_get_client_cert_chain_request_at(uid.as_bytes(), key.as_bytes(), 1)?,
        )?;
        let Value::Array(certificates) = decode(&response)? else {
            return Err(Error::CertificateChain);
        };
        let certificates = certificates
            .into_iter()
            .map(|certificate| match certificate {
                Value::Binary(bytes) if !bytes.is_empty() => Ok(bytes),
                _ => Err(Error::CertificateChain),
            })
            .collect::<Result<Vec<_>>>()?;
        if certificates.is_empty() {
            return Err(Error::CertificateChain);
        }
        Ok(certificates)
    }

    fn load_backup_puks_for_role(
        &self,
        host: &PinnedHost,
        credential: &LocatedBackupCredential,
        verified: &foks_verify::VerifiedUserState,
        role: Role,
    ) -> Result<Vec<UserPrivateKey>> {
        let role_key = verified.shared_key(role).ok_or(Error::UserBinding(
            "requested backup PUK role is not in the user chain",
        ))?;
        let request = encode_get_puk_for_role_request(role, credential.key.entity_id().as_bytes())?;
        let bytes = self.call_with_pkcs8_material(
            host,
            &host.user,
            &request,
            credential.key.expose_signing_key_pkcs8()?,
            &credential.certificate_chain,
        )?;
        let parcel = PukParcel::decode(&bytes)?;
        let sender = verified
            .devices()
            .iter()
            .find(|device| device.id == parcel.sender)
            .ok_or(Error::UserBinding("PUK parcel sender is not enrolled"))?;
        let clear = open_puk_parcel_with_for_role(
            &parcel,
            &credential.key,
            &sender.hepk,
            &role_key.verify_key,
            &role_key.hepk,
            role_key.generation,
            host.host_id(),
            role,
        )?;
        Ok(
            open_puk_seed_chain(clear, &parcel, verified.uid(), host.host_id())?
                .into_iter()
                .map(|key| UserPrivateKey {
                    role: key.role,
                    generation: key.generation,
                    seed: key.into_seed(),
                })
                .collect(),
        )
    }
}

fn credential_already_enrolled(
    devices: &[VerifiedDevice],
    expected: &foks_crypto::DevicePublicMaterial,
    role: Role,
) -> Result<bool> {
    let Some(enrolled) = devices.iter().find(|device| device.id == expected.id) else {
        return Ok(false);
    };
    if enrolled.role != role || enrolled.hepk != expected.hepk || enrolled.subkey.is_some() {
        return Err(Error::KeyBinding(
            "existing recovery device does not match the requested credential",
        ));
    }
    Ok(true)
}

fn current_milliseconds() -> Result<u64> {
    use std::time::{SystemTime, UNIX_EPOCH};
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| Error::CredentialBinding("system clock precedes Unix epoch"))?;
    u64::try_from(elapsed.as_millis())
        .map_err(|_| Error::CredentialBinding("system clock timestamp overflow"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use foks_proto::SecretSeed;

    #[test]
    fn mutation_retry_requires_the_same_authenticated_credential_binding() {
        let expected = derive_device_public(&SecretSeed::new([7; 32])).unwrap();
        assert!(!credential_already_enrolled(&[], &expected, Role::OWNER).unwrap());
        let enrolled = VerifiedDevice {
            id: expected.id.clone(),
            role: Role::OWNER,
            hepk: expected.hepk.clone(),
            subkey: None,
        };
        assert!(credential_already_enrolled(
            std::slice::from_ref(&enrolled),
            &expected,
            Role::OWNER,
        )
        .unwrap());

        let mut wrong_role = enrolled.clone();
        wrong_role.role = Role::ADMIN;
        assert!(matches!(
            credential_already_enrolled(&[wrong_role], &expected, Role::OWNER),
            Err(Error::KeyBinding(_))
        ));

        let mut wrong_hepk = enrolled;
        wrong_hepk.hepk = derive_device_public(&SecretSeed::new([8; 32]))
            .unwrap()
            .hepk;
        assert!(matches!(
            credential_already_enrolled(&[wrong_hepk], &expected, Role::OWNER),
            Err(Error::KeyBinding(_))
        ));

        let mut unexpected_subkey = VerifiedDevice {
            id: expected.id.clone(),
            role: Role::OWNER,
            hepk: expected.hepk.clone(),
            subkey: None,
        };
        unexpected_subkey.subkey =
            Some(foks_crypto::derive_subkey_id(&SecretSeed::new([9; 32])).unwrap());
        assert!(matches!(
            credential_already_enrolled(&[unexpected_subkey], &expected, Role::OWNER),
            Err(Error::KeyBinding(_))
        ));
    }
}
