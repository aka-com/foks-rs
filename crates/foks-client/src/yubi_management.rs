//! Encrypted Yubi PIV management-key lifecycle.

use foks_crypto::{derive_shared_verify_key, open_yubi_management_key, seal_yubi_management_key};
use foks_proto::{
    EntityId, Role, RoleType, YubiCardId, YubiEncryptedManagementKey, YubiManagementKeyBoxPayload,
    ENTITY_PUK_VERIFY,
};
use foks_snowpack::{decode, Value};
use zeroize::Zeroizing;

use crate::{
    random_bytes, DeviceCredential, Error, FoksClient, PinnedHost, Result, UserPrivateKey,
    YubiCredential,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum YubiEnvelopeRefresh {
    Fresh,
    Reencrypted,
}

pub struct RecoveredYubiManagementKey {
    pub management_key: Zeroizing<[u8; 24]>,
    pub card: YubiCardId,
    pub slot: u64,
    pub yubi_id: EntityId,
    pub puk_role: Role,
    pub puk_generation: u64,
}

pub fn encrypt_yubi_management_key(
    puk: &UserPrivateKey,
    yubi_id: &EntityId,
    card: YubiCardId,
    slot: u64,
    management_key: &[u8; 24],
) -> Result<YubiEncryptedManagementKey> {
    if puk.role.kind() < RoleType::Admin || puk.generation == 0 {
        return Err(Error::KeyBinding(
            "Yubi management keys require an administrator-or-owner PUK",
        ));
    }
    let expected = derive_shared_verify_key(&puk.seed, ENTITY_PUK_VERIFY)?;
    if expected.entity_type() != ENTITY_PUK_VERIFY {
        return Err(Error::KeyBinding("Yubi management-key PUK is invalid"));
    }
    let payload = YubiManagementKeyBoxPayload {
        management_key: *management_key,
        card,
        slot,
        yubi_id: yubi_id.clone(),
    };
    Ok(YubiEncryptedManagementKey {
        yubi_id: yubi_id.clone(),
        secret_box: seal_yubi_management_key(&puk.seed, &payload, random_bytes()?)?,
        generation: puk.generation,
        role: puk.role,
    })
}

pub fn decrypt_yubi_management_key(
    envelope: &YubiEncryptedManagementKey,
    puks: &[UserPrivateKey],
) -> Result<RecoveredYubiManagementKey> {
    let puk = puks
        .iter()
        .find(|puk| puk.role == envelope.role && puk.generation == envelope.generation)
        .ok_or(Error::KeyBinding(
            "encrypted Yubi management key has no matching loaded PUK",
        ))?;
    let payload = open_yubi_management_key(&puk.seed, &envelope.secret_box)?;
    if payload.yubi_id != envelope.yubi_id {
        return Err(Error::CredentialBinding(
            "Yubi management-key envelope identity changed",
        ));
    }
    Ok(RecoveredYubiManagementKey {
        management_key: Zeroizing::new(payload.management_key),
        card: payload.card.clone(),
        slot: payload.slot,
        yubi_id: payload.yubi_id.clone(),
        puk_role: envelope.role,
        puk_generation: envelope.generation,
    })
}

impl FoksClient {
    pub fn put_yubi_management_key(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
        value: &YubiEncryptedManagementKey,
    ) -> Result<()> {
        self.call_void(
            host,
            &host.user,
            &foks_rpc::encode_put_yubi_management_key_request(value)?,
            credential,
        )
    }

    pub fn put_yubi_management_key_yubi(
        &self,
        host: &PinnedHost,
        credential: &YubiCredential<'_>,
        value: &YubiEncryptedManagementKey,
    ) -> Result<()> {
        self.call_void_with_material(
            host,
            &host.user,
            &foks_rpc::encode_put_yubi_management_key_request(value)?,
            &credential.subkey_seed,
            &credential.certificate_chain,
        )
    }

    pub fn get_yubi_management_key(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
        parent: &EntityId,
    ) -> Result<YubiEncryptedManagementKey> {
        let response = self.call(
            host,
            &host.user,
            &foks_rpc::encode_get_yubi_management_key_request(parent)?,
            Some(credential),
        )?;
        decode_management_key_for_parent(&response, parent)
    }

    pub fn get_yubi_management_key_yubi(
        &self,
        host: &PinnedHost,
        credential: &YubiCredential<'_>,
        parent: &EntityId,
    ) -> Result<YubiEncryptedManagementKey> {
        let response = self.call_with_material(
            host,
            &host.user,
            &foks_rpc::encode_get_yubi_management_key_request(parent)?,
            &credential.subkey_seed,
            &credential.certificate_chain,
        )?;
        decode_management_key_for_parent(&response, parent)
    }

    pub fn get_all_yubi_management_keys(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
    ) -> Result<Vec<YubiEncryptedManagementKey>> {
        let response = self.call(
            host,
            &host.user,
            &foks_rpc::encode_get_all_yubi_management_keys_request()?,
            Some(credential),
        )?;
        decode_management_key_list(&response)
    }

    pub fn get_all_yubi_management_keys_yubi(
        &self,
        host: &PinnedHost,
        credential: &YubiCredential<'_>,
    ) -> Result<Vec<YubiEncryptedManagementKey>> {
        let response = self.call_with_material(
            host,
            &host.user,
            &foks_rpc::encode_get_all_yubi_management_keys_request()?,
            &credential.subkey_seed,
            &credential.certificate_chain,
        )?;
        decode_management_key_list(&response)
    }

    /// Reencrypts a stored envelope after a PUK rotation. The server update is
    /// monotonic in PUK generation, so replaying this idempotently is safe for
    /// the durable scheduler.
    pub fn refresh_yubi_management_key(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
        parent: &EntityId,
        current_puk: &UserPrivateKey,
        loaded_puks: &[UserPrivateKey],
    ) -> Result<YubiEnvelopeRefresh> {
        let stored = self.get_yubi_management_key(host, credential, parent)?;
        if stored.role == current_puk.role && stored.generation == current_puk.generation {
            // A matching public generation is not sufficient: another active
            // administrator can idempotently replace the opaque envelope at
            // that generation. Refuse to call it fresh unless the loaded PUK
            // authenticates the payload and its parent binding.
            let _ = decrypt_yubi_management_key(&stored, loaded_puks)?;
            return Ok(YubiEnvelopeRefresh::Fresh);
        }
        if stored.role != current_puk.role || stored.generation > current_puk.generation {
            return Err(Error::KeyBinding(
                "Yubi management-key envelope has an incompatible PUK version",
            ));
        }
        let recovered = decrypt_yubi_management_key(&stored, loaded_puks)?;
        let refreshed = encrypt_yubi_management_key(
            current_puk,
            &recovered.yubi_id,
            recovered.card,
            recovered.slot,
            &recovered.management_key,
        )?;
        self.put_yubi_management_key(host, credential, &refreshed)?;
        Ok(YubiEnvelopeRefresh::Reencrypted)
    }
}

fn decode_management_key_for_parent(
    bytes: &[u8],
    expected_parent: &EntityId,
) -> Result<YubiEncryptedManagementKey> {
    let value = YubiEncryptedManagementKey::decode(bytes)?;
    if &value.yubi_id != expected_parent {
        return Err(Error::CredentialBinding(
            "Yubi management-key response belongs to another parent",
        ));
    }
    Ok(value)
}

fn decode_management_key_list(bytes: &[u8]) -> Result<Vec<YubiEncryptedManagementKey>> {
    match decode(bytes)? {
        Value::Null => Ok(Vec::new()),
        Value::Array(values) => values
            .iter()
            .map(YubiEncryptedManagementKey::from_value)
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into),
        _ => Err(Error::CredentialBinding(
            "Yubi management-key list has the wrong shape",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use foks_proto::SecretSeed;

    #[test]
    fn management_key_envelope_binds_card_slot_parent_and_puk_version() {
        let puk = UserPrivateKey {
            role: Role::OWNER,
            generation: 3,
            seed: SecretSeed::new([7; 32]),
        };
        let mut id = vec![foks_proto::ENTITY_YUBI];
        id.extend_from_slice(&[2; 33]);
        let id = EntityId::from_bytes(id).unwrap();
        let envelope = encrypt_yubi_management_key(
            &puk,
            &id,
            YubiCardId {
                name: b"mock".to_vec(),
                serial: 9,
            },
            0x82,
            &[5; 24],
        )
        .unwrap();
        let recovered = decrypt_yubi_management_key(&envelope, &[puk]).unwrap();
        assert_eq!(recovered.management_key.as_slice(), &[5; 24]);
        assert_eq!(recovered.card.serial, 9);
        assert_eq!(recovered.slot, 0x82);
        assert_eq!(recovered.yubi_id, id);
    }

    #[test]
    fn single_management_key_response_is_bound_to_the_requested_parent() {
        let mut first = vec![foks_proto::ENTITY_YUBI];
        first.extend_from_slice(&[2; 33]);
        let first = EntityId::from_bytes(first).unwrap();
        let mut second = vec![foks_proto::ENTITY_YUBI];
        second.extend_from_slice(&[3; 33]);
        let second = EntityId::from_bytes(second).unwrap();
        let encoded = YubiEncryptedManagementKey {
            yubi_id: first,
            secret_box: foks_proto::SecretBox {
                nonce: [4; 16],
                ciphertext: vec![5; 16],
            },
            generation: 1,
            role: Role::OWNER,
        }
        .encoded()
        .unwrap();
        assert!(matches!(
            decode_management_key_for_parent(&encoded, &second),
            Err(Error::CredentialBinding(_))
        ));
    }
}
