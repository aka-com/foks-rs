use std::collections::BTreeMap;

use foks_crypto::verify_typed;
use foks_proto::{ChangeMetadata, EntityId, Hepk, Role, UserMemberKeys, LINK_OUTER_V1_TYPE_ID};

use crate::{find_hepk, Error, Result, UserTransitionRule, VerifiedDevice, VerifiedSharedKey};

pub(super) struct UserReplayState {
    devices: BTreeMap<Vec<u8>, VerifiedDevice>,
    shared_keys: BTreeMap<Role, VerifiedSharedKey>,
}

impl UserReplayState {
    pub(super) fn from_eldest(device: VerifiedDevice, shared_key: VerifiedSharedKey) -> Self {
        Self {
            devices: BTreeMap::from([(device.id.as_bytes().to_vec(), device)]),
            shared_keys: BTreeMap::from([(shared_key.role, shared_key)]),
        }
    }

    pub(super) fn from_verified(
        devices: &[VerifiedDevice],
        shared_keys: &[VerifiedSharedKey],
    ) -> Self {
        Self {
            devices: devices
                .iter()
                .cloned()
                .map(|device| (device.id.as_bytes().to_vec(), device))
                .collect(),
            shared_keys: shared_keys
                .iter()
                .cloned()
                .map(|key| (key.role, key))
                .collect(),
        }
    }

    pub(super) fn replay(
        &mut self,
        link: &foks_proto::UserLink,
        change: &foks_proto::UserGroupChange,
        hepks: &[Hepk],
        expected_host: &EntityId,
    ) -> Result<()> {
        if change.changes.len() > 1 {
            return Err(invalid_transition(
                change,
                UserTransitionRule::MultipleMemberChanges,
            ));
        }
        let signer = self
            .devices
            .get(change.signer.as_bytes())
            .cloned()
            .ok_or_else(|| invalid_transition(change, UserTransitionRule::UnknownSigner))?;
        let rotated = validate_shared_key_rotations(change, hepks, &self.shared_keys)?;
        let (added_device, provisioning_subkey) = validate_provisioning(
            change,
            hepks,
            &signer,
            &self.devices,
            &self.shared_keys,
            &rotated,
        )?;
        verify_transition_signatures(
            link,
            &signer,
            &rotated,
            added_device.as_ref(),
            provisioning_subkey.as_ref(),
        )?;
        self.apply_transition(change, expected_host, &signer, &rotated, added_device)
    }

    pub(super) fn is_complete(&self) -> bool {
        !self.devices.is_empty() && self.shared_keys.contains_key(&Role::OWNER)
    }

    pub(super) fn into_parts(self) -> (Vec<VerifiedDevice>, Vec<VerifiedSharedKey>) {
        (
            self.devices.into_values().collect(),
            self.shared_keys.into_values().collect(),
        )
    }

    fn apply_transition(
        &mut self,
        change: &foks_proto::UserGroupChange,
        expected_host: &EntityId,
        signer: &VerifiedDevice,
        rotated: &[VerifiedSharedKey],
        added_device: Option<VerifiedDevice>,
    ) -> Result<()> {
        match change.changes.first() {
            Some(member) if member.role == Role::NONE => {
                self.apply_revocation(change, member, expected_host, signer, rotated)?;
            }
            Some(_) => {
                let device = added_device
                    .ok_or_else(|| invalid_transition(change, UserTransitionRule::Provisioning))?;
                self.devices.insert(device.id.as_bytes().to_vec(), device);
            }
            None => validate_standalone_change(change, signer, rotated, &self.shared_keys)?,
        }
        for key in rotated {
            self.shared_keys.insert(key.role, key.clone());
        }
        Ok(())
    }

    fn apply_revocation(
        &mut self,
        change: &foks_proto::UserGroupChange,
        member: &foks_proto::UserMemberChange,
        expected_host: &EntityId,
        signer: &VerifiedDevice,
        rotated: &[VerifiedSharedKey],
    ) -> Result<()> {
        if !matches!(member.keys, UserMemberKeys::None)
            || !change.metadata.is_empty()
            || member
                .scoped_host
                .as_ref()
                .is_some_and(|host| host != expected_host)
        {
            return Err(invalid_transition(change, UserTransitionRule::Revocation));
        }
        let target = self
            .devices
            .get(member.entity.as_bytes())
            .ok_or_else(|| invalid_transition(change, UserTransitionRule::Revocation))?;
        let self_revoke = target.id == signer.id;
        if self_revoke != rotated.is_empty()
            || (!self_revoke && signer.role != Role::OWNER)
            || (!self_revoke
                && self.shared_keys.keys().any(|role| {
                    *role <= target.role && !rotated.iter().any(|key| key.role == *role)
                }))
        {
            return Err(invalid_transition(change, UserTransitionRule::Revocation));
        }
        if target.role == Role::OWNER
            && self
                .devices
                .values()
                .filter(|device| device.role == Role::OWNER)
                .count()
                == 1
        {
            return Err(invalid_transition(change, UserTransitionRule::LastOwner));
        }
        self.devices.remove(member.entity.as_bytes());
        Ok(())
    }
}

fn validate_shared_key_rotations(
    change: &foks_proto::UserGroupChange,
    hepks: &[Hepk],
    shared_keys: &BTreeMap<Role, VerifiedSharedKey>,
) -> Result<Vec<VerifiedSharedKey>> {
    let added_role = change
        .changes
        .first()
        .filter(|member| member.role != Role::NONE)
        .map(|member| member.role);
    let mut rotated = Vec::with_capacity(change.shared_keys.len());
    let mut last_role = None;
    for key in &change.shared_keys {
        if key.role == Role::NONE
            || last_role.is_some_and(|role| role >= key.role)
            || match shared_keys.get(&key.role) {
                Some(current) => current
                    .generation
                    .checked_add(1)
                    .is_none_or(|next| key.generation != next),
                None => added_role != Some(key.role) || key.generation != 1,
            }
        {
            return Err(invalid_transition(
                change,
                UserTransitionRule::SharedKeyRotation,
            ));
        }
        last_role = Some(key.role);
        let hepk = find_hepk(hepks, key.hepk_fingerprint)?;
        if hepk.curve25519().is_none() {
            return Err(invalid_transition(
                change,
                UserTransitionRule::SharedKeyRotation,
            ));
        }
        rotated.push(VerifiedSharedKey {
            role: key.role,
            generation: key.generation,
            verify_key: key.verify_key.clone(),
            hepk,
        });
    }
    Ok(rotated)
}

fn validate_provisioning(
    change: &foks_proto::UserGroupChange,
    hepks: &[Hepk],
    signer: &VerifiedDevice,
    devices: &BTreeMap<Vec<u8>, VerifiedDevice>,
    shared_keys: &BTreeMap<Role, VerifiedSharedKey>,
    rotated: &[VerifiedSharedKey],
) -> Result<(Option<VerifiedDevice>, Option<EntityId>)> {
    let Some(member) = change
        .changes
        .first()
        .filter(|member| member.role != Role::NONE)
    else {
        return Ok((None, None));
    };
    if signer.role != Role::OWNER
        || member.scoped_host.is_some()
        || member.source_role != Role::NONE
        || devices.contains_key(member.entity.as_bytes())
        || !matches!(change.metadata.as_slice(), [ChangeMetadata::DeviceName(_)])
        || rotated.len() > 1
        || rotated.first().is_some_and(|key| key.role != member.role)
        || (!rotated.is_empty() && member.role >= signer.role)
        || (member.role < signer.role
            && !shared_keys.contains_key(&member.role)
            && rotated.is_empty())
    {
        return Err(invalid_transition(change, UserTransitionRule::Provisioning));
    }
    let UserMemberKeys::User {
        hepk_fingerprint,
        subkey,
    } = &member.keys
    else {
        return Err(invalid_transition(change, UserTransitionRule::Provisioning));
    };
    let valid_subkey = match member.entity.entity_type() {
        foks_proto::ENTITY_YUBI => subkey.is_some(),
        foks_proto::ENTITY_DEVICE
        | foks_proto::ENTITY_BACKUP_KEY
        | foks_proto::ENTITY_BOT_TOKEN_KEY => subkey.is_none(),
        _ => false,
    };
    if !valid_subkey {
        return Err(invalid_transition(change, UserTransitionRule::Provisioning));
    }
    let hepk = find_hepk(hepks, *hepk_fingerprint)?;
    if !user_member_hepk_matches(&member.entity, &hepk) {
        return Err(invalid_transition(change, UserTransitionRule::Provisioning));
    }
    Ok((
        Some(VerifiedDevice {
            id: member.entity.clone(),
            role: member.role,
            hepk,
            subkey: subkey.clone(),
        }),
        subkey.clone(),
    ))
}

pub(crate) fn user_member_hepk_matches(entity: &EntityId, hepk: &Hepk) -> bool {
    match entity.entity_type() {
        foks_proto::ENTITY_DEVICE
        | foks_proto::ENTITY_BACKUP_KEY
        | foks_proto::ENTITY_BOT_TOKEN_KEY => hepk.curve25519().is_some(),
        foks_proto::ENTITY_YUBI => hepk.p256().is_some(),
        _ => false,
    }
}

fn verify_transition_signatures(
    link: &foks_proto::UserLink,
    signer: &VerifiedDevice,
    rotated: &[VerifiedSharedKey],
    added_device: Option<&VerifiedDevice>,
    provisioning_subkey: Option<&EntityId>,
) -> Result<()> {
    let mut verifiers = rotated
        .iter()
        .map(|key| key.verify_key.clone())
        .collect::<Vec<_>>();
    if let Some(device) = added_device {
        if let Some(subkey) = provisioning_subkey {
            verifiers.push(subkey.clone());
        }
        verifiers.push(device.id.clone());
    }
    verifiers.push(signer.id.clone());
    if verifiers.len() != link.signatures().len() {
        return Err(Error::UserSignatureCount(link.signatures().len()));
    }
    for (index, verifier) in verifiers.iter().enumerate() {
        verify_typed(
            verifier,
            &link.signatures()[index],
            LINK_OUTER_V1_TYPE_ID,
            &link.signing_bytes(index)?,
        )?;
    }
    Ok(())
}

fn validate_standalone_change(
    change: &foks_proto::UserGroupChange,
    signer: &VerifiedDevice,
    rotated: &[VerifiedSharedKey],
    shared_keys: &BTreeMap<Role, VerifiedSharedKey>,
) -> Result<()> {
    let valid = if rotated.is_empty() {
        matches!(change.metadata.as_slice(), [ChangeMetadata::Username(_)])
    } else {
        let upper = rotated
            .last()
            .ok_or_else(|| invalid_transition(change, UserTransitionRule::StandaloneChange))?
            .role;
        signer.role == Role::OWNER
            && change.metadata.is_empty()
            && !shared_keys
                .keys()
                .any(|role| *role <= upper && !rotated.iter().any(|key| key.role == *role))
    };
    if !valid {
        return Err(invalid_transition(
            change,
            UserTransitionRule::StandaloneChange,
        ));
    }
    Ok(())
}

fn invalid_transition(change: &foks_proto::UserGroupChange, rule: UserTransitionRule) -> Error {
    Error::UserTransition {
        seqno: change.seqno,
        rule,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use foks_proto::{SecretSeed, UserChain, UserLink, UserMemberKeys};

    const USER_DIR: &str = "../foks-snowpack/tests/fixtures/foks-v0.1.9/user";
    const MUTATION_DIR: &str = "../foks-snowpack/tests/fixtures/foks-v0.1.9/user-mutations";

    fn user_fixture(name: &str) -> Vec<u8> {
        std::fs::read(format!("{USER_DIR}/{name}")).unwrap()
    }

    fn mutation_fixture(name: &str) -> Vec<u8> {
        std::fs::read(format!("{MUTATION_DIR}/{name}")).unwrap()
    }

    fn state_before_backup() -> (UserReplayState, EntityId) {
        let chain = UserChain::decode(&user_fixture("user-chain.snowp")).unwrap();
        let eldest = chain.links[0].decode_eldest().unwrap();
        let rotation = chain.links[2].decode_group_change().unwrap();
        let current = &rotation.shared_keys[0];
        (
            UserReplayState::from_eldest(
                VerifiedDevice {
                    id: eldest.member,
                    role: Role::OWNER,
                    hepk: find_hepk(&chain.hepks, eldest.member_hepk_fingerprint).unwrap(),
                    subkey: None,
                },
                VerifiedSharedKey {
                    role: current.role,
                    generation: current.generation,
                    verify_key: current.verify_key.clone(),
                    hepk: find_hepk(&chain.hepks, current.hepk_fingerprint).unwrap(),
                },
            ),
            eldest.host,
        )
    }

    #[test]
    fn official_backup_and_recovered_device_transitions_replay() {
        let (mut state, host) = state_before_backup();
        let enroll = UserLink::decode(&mutation_fixture("backup-enroll-link.snowp")).unwrap();
        let enroll_change = enroll.decode_group_change().unwrap();
        let backup_hepk = Hepk::decode(&mutation_fixture("backup-hepk.snowp")).unwrap();
        state
            .replay(&enroll, &enroll_change, &[backup_hepk], &host)
            .unwrap();
        assert!(state.devices.values().any(|device| {
            device.id.entity_type() == foks_proto::ENTITY_BACKUP_KEY && device.role == Role::OWNER
        }));

        let recover = UserLink::decode(&mutation_fixture("backup-recover-link.snowp")).unwrap();
        let recover_change = recover.decode_group_change().unwrap();
        let replacement_seed = SecretSeed::new(
            mutation_fixture("backup-recover-device-seed.bin")
                .try_into()
                .unwrap(),
        );
        let replacement = foks_crypto::derive_device_public(&replacement_seed).unwrap();
        state
            .replay(&recover, &recover_change, &[replacement.hepk], &host)
            .unwrap();
        assert!(state.devices.contains_key(replacement.id.as_bytes()));
    }

    #[test]
    fn backup_provisioning_rejects_a_subkey() {
        let (state, _) = state_before_backup();
        let link = UserLink::decode(&mutation_fixture("backup-enroll-link.snowp")).unwrap();
        let mut change = link.decode_group_change().unwrap();
        let UserMemberKeys::User { subkey, .. } = &mut change.changes[0].keys else {
            panic!("backup fixture must contain user member keys");
        };
        let seed = SecretSeed::new([7; 32]);
        *subkey = Some(foks_crypto::derive_subkey_id(&seed).unwrap());
        let backup_hepk = Hepk::decode(&mutation_fixture("backup-hepk.snowp")).unwrap();
        let signer = state.devices.get(change.signer.as_bytes()).unwrap();
        let rotated = validate_shared_key_rotations(
            &change,
            std::slice::from_ref(&backup_hepk),
            &state.shared_keys,
        )
        .unwrap();
        assert!(matches!(
            validate_provisioning(
                &change,
                &[backup_hepk],
                signer,
                &state.devices,
                &state.shared_keys,
                &rotated,
            ),
            Err(Error::UserTransition {
                rule: UserTransitionRule::Provisioning,
                ..
            })
        ));
    }

    #[test]
    fn backup_members_require_the_software_hepk_suite() {
        let backup_id =
            match foks_snowpack::decode(&mutation_fixture("backup-entity-id.snowp")).unwrap() {
                foks_snowpack::Value::Binary(bytes) => EntityId::from_bytes(bytes).unwrap(),
                _ => panic!("backup fixture must contain an EntityID"),
            };
        let backup_hepk = Hepk::decode(&mutation_fixture("backup-hepk.snowp")).unwrap();
        let yubi_hepk =
            Hepk::decode(&std::fs::read(format!("{USER_DIR}/yubi/yubi-hepk.snowp")).unwrap())
                .unwrap();
        assert!(user_member_hepk_matches(&backup_id, &backup_hepk));
        assert!(!user_member_hepk_matches(&backup_id, &yubi_hepk));
    }
}
