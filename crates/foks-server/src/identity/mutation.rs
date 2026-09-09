use foks_proto::{
    DecodedProvisionDeviceArgument, DecodedRevokeDeviceArgument, EntityId, Hepk, Role, RoleType,
    SeedChainBox, SharedKeyBoxSet, UserMemberKeys,
};

use crate::{Error, Result};

pub(crate) enum Argument {
    Provision(DecodedProvisionDeviceArgument),
    Revoke(DecodedRevokeDeviceArgument),
}

impl Argument {
    pub(crate) fn link(&self) -> &foks_proto::UserLink {
        match self {
            Self::Provision(argument) => &argument.link,
            Self::Revoke(argument) => &argument.link,
        }
    }
}

pub(crate) struct AddedCredential {
    pub device_id: Vec<u8>,
    pub self_token: [u8; 17],
    pub hepk_fingerprint: [u8; 32],
    pub exact_hepk: Vec<u8>,
    pub exact_name: Vec<u8>,
    pub role: Role,
    pub subkey_id: Option<Vec<u8>>,
    pub exact_subkey_box: Option<Vec<u8>>,
    pub yubi_pq_hint: Option<(u8, [u8; 32])>,
}

pub(crate) struct SharedKey {
    pub role: Role,
    pub generation: u64,
    pub verify_key: Vec<u8>,
    pub exact_hepk: Vec<u8>,
}

pub(crate) struct Parcel {
    pub device_id: Vec<u8>,
    pub role: Role,
    pub generation: u64,
    pub exact: Vec<u8>,
}

pub(crate) struct Command {
    pub uid: Vec<u8>,
    pub signer: Vec<u8>,
    pub sequence: u64,
    pub expected_tail_hash: [u8; 32],
    pub link_hash: [u8; 32],
    pub exact_link: Vec<u8>,
    pub next_tree_location: [u8; 32],
    pub added: Option<AddedCredential>,
    pub revoked: Option<Vec<u8>>,
    pub shared_keys: Vec<SharedKey>,
    pub parcels: Vec<Parcel>,
    pub seed_chain: Vec<SeedChainBox>,
    pub passphrase: Option<foks_proto::PassphraseUpdateArgument>,
    pub user_settings: Option<UserSettingsMutation>,
}

pub(crate) struct UserSettingsMutation {
    pub signer: Vec<u8>,
    pub sequence: u64,
    pub previous: Option<[u8; 32]>,
    pub root: foks_proto::TreeRoot,
    pub next_tree_location: [u8; 32],
    pub link_hash: [u8; 32],
    pub exact_link: Vec<u8>,
    pub info: foks_proto::PassphraseInfo,
}

pub(crate) fn validate(
    authority: &foks_server_db::UserAuthoritySnapshot,
    host: &EntityId,
    principal: &[u8],
    argument: Argument,
    signed_root: foks_proto::TreeRoot,
) -> Result<Command> {
    let is_revoke_request = matches!(&argument, Argument::Revoke(_));
    if let Argument::Provision(provision) = &argument {
        for device in authority.devices.iter().filter(|device| device.active) {
            let existing = foks_proto::DeviceLabelNameAndCommitmentKey::decode(&device.exact_name)?;
            if existing.label == provision.device_name.label {
                return Err(Error::Signup("device label is already enrolled"));
            }
        }
    }
    let provision_self_token = match &argument {
        Argument::Provision(argument) => Some(argument.self_token),
        Argument::Revoke(_) => None,
    };
    let (
        link,
        next_tree_location,
        hepks,
        boxes,
        seed_chain,
        exact_name,
        passphrase,
        subkey_box,
        yubi_pq_hint,
    ) = match &argument {
        Argument::Provision(argument) => (
            &argument.link,
            argument.next_tree_location,
            &argument.hepks,
            &argument.puk_boxes,
            Vec::new(),
            Some(argument.device_name.encoded()?),
            None,
            argument
                .subkey_box
                .as_ref()
                .map(foks_proto::HybridBox::encoded)
                .transpose()?,
            argument
                .yubi_pq_hint
                .as_ref()
                .map(|hint| -> Result<(u8, [u8; 32])> {
                    Ok((
                        u8::try_from(hint.slot).map_err(|_| Error::Signup("Yubi PQ slot range"))?,
                        hint.id,
                    ))
                })
                .transpose()?,
        ),
        Argument::Revoke(argument) => (
            &argument.link,
            argument.next_tree_location,
            &argument.hepks,
            &argument.puk_boxes,
            argument.seed_chain.clone(),
            None,
            argument.passphrase.clone(),
            None,
            None,
        ),
    };
    let uid = EntityId::from_bytes(authority.uid.clone())?;
    let devices = authority
        .devices
        .iter()
        .filter(|device| device.active)
        .map(|device| {
            Ok(foks_verify::VerifiedDevice {
                id: EntityId::from_bytes(device.device_id.clone())?,
                role: decode_role(device.role_type, device.visibility)?,
                hepk: Hepk::decode(&device.exact_hepk)?,
                subkey: device
                    .subkey_id
                    .clone()
                    .map(EntityId::from_bytes)
                    .transpose()?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let shared_key_history = decode_shared_keys(&authority.shared_keys)?;
    let shared_keys = latest_shared_keys(&shared_key_history);
    let expected_sequence = authority
        .chain_sequence
        .checked_add(1)
        .ok_or(Error::Signup("user chain sequence overflow"))?;
    let verified = foks_verify::verify_user_transition(
        link,
        hepks,
        &uid,
        host,
        expected_sequence,
        authority.chain_tail_hash,
        signed_root.clone(),
        next_tree_location,
        &devices,
        &shared_keys,
        &shared_key_history,
    )?;
    if verified.change.signer.as_bytes() != principal {
        return Err(Error::Signup(
            "mutation signer does not match TLS principal",
        ));
    }
    let member = verified.change.changes.first();
    let (added, revoked) = match member {
        Some(member) if member.role == Role::NONE => {
            (None, Some(member.entity.as_bytes().to_vec()))
        }
        Some(member) => {
            let UserMemberKeys::User {
                hepk_fingerprint,
                ref subkey,
            } = member.keys
            else {
                return Err(Error::Signup("unsupported provisioned credential"));
            };
            let hepk = hepk_by_fingerprint(hepks, hepk_fingerprint)?;
            let exact_name = exact_name
                .clone()
                .ok_or(Error::Signup("provisioning request has no device name"))?;
            (
                Some(AddedCredential {
                    device_id: member.entity.as_bytes().to_vec(),
                    self_token: provision_self_token
                        .ok_or(Error::Signup("provisioning request has no self token"))?,
                    hepk_fingerprint,
                    exact_hepk: hepk.encoded()?,
                    exact_name,
                    role: member.role,
                    subkey_id: subkey.as_ref().map(|subkey| subkey.as_bytes().to_vec()),
                    exact_subkey_box: subkey_box.clone(),
                    yubi_pq_hint,
                }),
                None,
            )
        }
        None => (None, None),
    };
    let introduced = verified
        .change
        .shared_keys
        .iter()
        .map(|key| {
            Ok(SharedKey {
                role: key.role,
                generation: key.generation,
                verify_key: key.verify_key.as_bytes().to_vec(),
                exact_hepk: hepk_by_fingerprint(hepks, key.hepk_fingerprint)?.encoded()?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    if is_revoke_request && !introduced.is_empty() {
        validate_rotation_box_gameplan(&devices, revoked.as_deref(), &introduced, boxes)?;
    }
    let exact_boxes = boxes.encoded();
    let parcels = boxes
        .boxes
        .iter()
        .map(|boxed| Parcel {
            device_id: boxed.target.entity.as_bytes().to_vec(),
            role: boxed.role,
            generation: boxed.generation,
            exact: exact_boxes.clone(),
        })
        .collect();
    let exact_link = link.encoded()?;
    let user_settings = passphrase
        .as_ref()
        .and_then(|passphrase| {
            passphrase
                .user_settings_link
                .as_ref()
                .map(|link| (passphrase, link))
        })
        .map(|(passphrase, link)| {
            let decoded = link.link.decode_generic()?;
            let foks_proto::GenericLinkPayload::UserSettings(info) = decoded.payload else {
                return Err(Error::Signup("passphrase annex generic payload mismatch"));
            };
            let next_wire = foks_snowpack::encode(&foks_snowpack::Value::Binary(
                link.next_tree_location.to_vec(),
            ))?;
            if decoded.entity.as_bytes() != authority.uid {
                return Err(Error::Signup("passphrase settings entity mismatch"));
            }
            if decoded.host != *host {
                return Err(Error::Signup("passphrase settings host mismatch"));
            }
            if decoded.signer.as_bytes() != principal {
                return Err(Error::Signup("passphrase settings signer mismatch"));
            }
            // ChangePassphraseArg omits the salt, so revoke-annex decoding is
            // intentionally unbound here. The database transaction validates
            // the settings salt against the authoritative PPE snapshot after
            // applying the update.
            if info.generation != passphrase.generation
                || info.stretch_version != passphrase.stretch_version.protocol_value()
            {
                return Err(Error::Signup("passphrase settings payload mismatch"));
            }
            if link.link.signatures().len() != 1
                || foks_crypto::verify_typed(
                    &decoded.signer,
                    &link.link.signatures()[0],
                    foks_proto::LINK_OUTER_V1_TYPE_ID,
                    &link.link.signing_bytes(0)?,
                )
                .is_err()
            {
                return Err(Error::Signup("passphrase settings signature mismatch"));
            }
            if decoded.next_location_commitment
                != foks_crypto::prefixed_hash_signable(
                    foks_proto::TREE_LOCATION_TYPE_ID,
                    &next_wire,
                )?
            {
                return Err(Error::Signup("passphrase settings location mismatch"));
            }
            let exact_link = link.link.encoded()?;
            Ok(UserSettingsMutation {
                signer: decoded.signer.into_bytes(),
                sequence: decoded.sequence,
                previous: decoded.previous,
                root: decoded.root,
                next_tree_location: link.next_tree_location,
                link_hash: foks_crypto::prefixed_hash_signable(
                    foks_proto::LINK_OUTER_TYPE_ID,
                    &exact_link,
                )?,
                exact_link,
                info,
            })
        })
        .transpose()?;
    if passphrase.is_some() && user_settings.is_none() {
        return Err(Error::Signup(
            "configured passphrase annex omitted its UserSettings link",
        ));
    }
    Ok(Command {
        uid: authority.uid.clone(),
        signer: verified.change.signer.as_bytes().to_vec(),
        sequence: expected_sequence,
        expected_tail_hash: authority.chain_tail_hash,
        link_hash: foks_crypto::prefixed_hash_signable(
            foks_proto::LINK_OUTER_TYPE_ID,
            &exact_link,
        )?,
        exact_link,
        next_tree_location,
        added,
        revoked,
        shared_keys: introduced,
        parcels,
        seed_chain,
        passphrase,
        user_settings,
    })
}

/// Mirrors go-foks's `ComputeRotateNewBoxGameplan`: every active credential
/// that can read a rotated role must receive that role's next generation,
/// with no duplicate or surplus parcels. An empty shared-key change is the
/// intentional Go self-revoke shape and is handled by the caller.
fn validate_rotation_box_gameplan(
    devices: &[foks_verify::VerifiedDevice],
    revoked: Option<&[u8]>,
    rotated: &[SharedKey],
    boxes: &SharedKeyBoxSet,
) -> Result<()> {
    let expected = devices
        .iter()
        .filter(|device| revoked != Some(device.id.as_bytes()))
        .flat_map(|device| {
            rotated
                .iter()
                .filter(|key| device.role >= key.role)
                .map(|key| (device.id.as_bytes().to_vec(), key.role, key.generation))
        })
        .collect::<std::collections::BTreeSet<_>>();
    let actual = boxes
        .boxes
        .iter()
        .map(|boxed| {
            if boxed.target.host.is_some()
                || boxed.target.role != Role::NONE
                || boxed.target.generation != 0
            {
                return Err(Error::Signup("invalid local PUK box target"));
            }
            Ok((
                boxed.target.entity.as_bytes().to_vec(),
                boxed.role,
                boxed.generation,
            ))
        })
        .collect::<Result<std::collections::BTreeSet<_>>>()?;
    if actual.len() != boxes.boxes.len() || actual != expected {
        return Err(Error::Signup(
            "PUK rotation box set does not match the active roster",
        ));
    }
    Ok(())
}

fn decode_shared_keys(
    keys: &[foks_server_db::UserSharedKeySnapshot],
) -> Result<Vec<foks_verify::VerifiedSharedKey>> {
    keys.iter()
        .map(|key| {
            let role = decode_role(key.role_type, key.visibility)?;
            Ok(foks_verify::VerifiedSharedKey {
                role,
                generation: key.generation,
                verify_key: EntityId::from_bytes(key.verify_key.clone())?,
                hepk: Hepk::decode(&key.exact_hepk)?,
            })
        })
        .collect()
}

fn latest_shared_keys(
    keys: &[foks_verify::VerifiedSharedKey],
) -> Vec<foks_verify::VerifiedSharedKey> {
    let mut latest = std::collections::BTreeMap::new();
    for key in keys {
        match latest.entry(key.role) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(key.clone());
            }
            std::collections::btree_map::Entry::Occupied(mut entry)
                if key.generation > entry.get().generation =>
            {
                entry.insert(key.clone());
            }
            std::collections::btree_map::Entry::Occupied(_) => {}
        }
    }
    latest.into_values().collect()
}

fn decode_role(kind: u64, visibility: i64) -> Result<Role> {
    match kind {
        0 => Ok(Role::NONE),
        1 => Ok(Role::member(
            i16::try_from(visibility).map_err(|_| Error::Signup("stored role visibility"))?,
        )),
        2 if visibility == 0 => Ok(Role::ADMIN),
        3 if visibility == 0 => Ok(Role::OWNER),
        _ => Err(Error::Signup("stored role")),
    }
}

pub(crate) fn role_parts(role: Role) -> (u64, i64) {
    (
        role.protocol_value(),
        match role.kind() {
            RoleType::Member => i64::from(role.visibility().unwrap_or_default()),
            _ => 0,
        },
    )
}

fn hepk_by_fingerprint(hepks: &[Hepk], expected: [u8; 32]) -> Result<&Hepk> {
    hepks
        .iter()
        .find(|hepk| foks_crypto::hepk_fingerprint(hepk).ok() == Some(expected))
        .ok_or(Error::Signup("mutation HEPK is missing"))
}

#[cfg(test)]
mod tests {
    use foks_crypto::{
        derive_device_public, derive_shared_public, seal_software_puk_boxes_mixed,
        PukBoxRandomness, SoftwarePukBoxInput, SoftwarePukBoxSetRandomness,
    };
    use foks_proto::{EntityId, Role, SecretSeed, SharedKeyBoxSet, ENTITY_HOST, ENTITY_PUK_VERIFY};

    use super::{validate_rotation_box_gameplan, SharedKey};

    #[test]
    fn rotated_puk_boxes_must_exactly_cover_the_post_revoke_roster() {
        let host = EntityId::from_bytes([vec![ENTITY_HOST], vec![0x11; 32]].concat()).unwrap();
        let sender_seed = SecretSeed::new([0x12; 32]);
        let receiver_a = derive_device_public(&SecretSeed::new([0x13; 32])).unwrap();
        let receiver_b = derive_device_public(&SecretSeed::new([0x14; 32])).unwrap();
        let new_puk = SecretSeed::new([0x15; 32]);
        let public_puk = derive_shared_public(&new_puk, ENTITY_PUK_VERIFY).unwrap();
        let devices = [&receiver_a, &receiver_b]
            .into_iter()
            .map(|device| foks_verify::VerifiedDevice {
                id: device.id.clone(),
                role: Role::OWNER,
                hepk: device.hepk.clone(),
                subkey: None,
            })
            .collect::<Vec<_>>();
        let inputs = [&receiver_a, &receiver_b].map(|receiver| SoftwarePukBoxInput {
            seed: &new_puk,
            generation: 2,
            role: Role::OWNER,
            receiver,
        });
        let boxes = seal_software_puk_boxes_mixed(
            &host,
            &sender_seed,
            [0x16; 16],
            &inputs,
            &[
                PukBoxRandomness {
                    kem_message: [0x17; 32],
                    nonce: [0x18; 16],
                },
                PukBoxRandomness {
                    kem_message: [0x19; 32],
                    nonce: [0x1a; 16],
                },
            ],
            SoftwarePukBoxSetRandomness {
                ephemeral_secret: [0x1b; 32],
                time: 1,
            },
        )
        .unwrap();
        let rotated = [SharedKey {
            role: Role::OWNER,
            generation: 2,
            verify_key: public_puk.verify_key.into_bytes(),
            exact_hepk: public_puk.hepk.encoded().unwrap(),
        }];

        validate_rotation_box_gameplan(&devices, None, &rotated, &boxes).unwrap();

        let empty = SharedKeyBoxSet::new([0x16; 16], Vec::new(), None).unwrap();
        assert!(validate_rotation_box_gameplan(&devices, None, &rotated, &empty).is_err());

        let survivor_only = SharedKeyBoxSet::new(
            boxes.box_id,
            vec![boxes.boxes[1].clone()],
            boxes.temp_dh_key.clone(),
        )
        .unwrap();
        validate_rotation_box_gameplan(
            &devices,
            Some(receiver_a.id.as_bytes()),
            &rotated,
            &survivor_only,
        )
        .unwrap();
    }
}
